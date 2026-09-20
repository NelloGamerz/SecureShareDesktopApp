#!/usr/bin/env node
/**
 * Asserts that every command the webview invokes exists in Rust.
 *
 * The webview's only way to reach Rust is `invoke("<name>", ...)`, and the
 * only way a name becomes reachable is by being listed in the
 * `generate_handler![...]` registry in `crates/desktop/src/lib.rs`. Nothing checks
 * that the two agree, so a typo or a forgotten registration is a runtime
 * `command not found` with no compile-time signal.
 *
 * That is exactly how `stop_websocket` came to be called by the webview on
 * sign-out while never being registered, so the socket outlived the session
 * that authenticated it.
 *
 * Scope: command *names* only. Argument-name and argument-type agreement
 * between the two sides is not checked here.
 *
 * Exit code 1 if an invoked command is not registered.
 */

import { readFileSync, readdirSync, statSync } from "node:fs";
import { join, relative, sep } from "node:path";
import { fileURLToPath } from "node:url";

const repoRoot = fileURLToPath(new URL("..", import.meta.url));
const registryPath = join(repoRoot, "crates", "desktop", "src", "lib.rs");
const frontendRoot = join(repoRoot, "src");

/** Reads a file, normalising line endings so offsets are stable. */
function readSource(path) {
  return readFileSync(path, "utf8").replace(/\r\n/g, "\n");
}

/** Every `.ts`/`.tsx` file under `dir`, excluding declaration files. */
function collectSources(dir) {
  const found = [];

  for (const entry of readdirSync(dir)) {
    const full = join(dir, entry);

    if (statSync(full).isDirectory()) {
      found.push(...collectSources(full));

      continue;
    }

    if (!/\.tsx?$/.test(entry) || /\.d\.ts$/.test(entry)) {
      continue;
    }

    found.push(full);
  }

  return found;
}

/** Command names listed in `generate_handler![...]`. */
function readRegisteredCommands(source) {
  const start = source.indexOf("generate_handler![");

  if (start === -1) {
    throw new Error(
      `could not find generate_handler![ in ${relative(repoRoot, registryPath)}`,
    );
  }

  const open = source.indexOf("[", start);
  const close = source.indexOf("]", open);

  if (close === -1) {
    throw new Error("generate_handler![ is not closed");
  }

  const body = source.slice(open + 1, close);
  const names = new Set();

  for (const raw of body.split(",")) {
    const path = raw.trim();

    if (!path) {
      continue;
    }

    // Entries are identifiers or `module::sub::command` paths. The registry
    // uses the last segment.
    const segments = path.split("::");
    const name = segments[segments.length - 1].trim();

    if (/^[A-Za-z_][A-Za-z0-9_]*$/.test(name)) {
      names.add(name);
    }
  }

  return names;
}

/**
 * Strips comments so a commented-out call is not mistaken for a live one.
 *
 * A line comment is only stripped when the `//` sits outside a string, so
 * `https://…` inside a literal survives.
 */
function stripComments(source) {
  const withoutBlockComments = source.replace(/\/\*[\s\S]*?\*\//g, (match) =>
    match.replace(/[^\n]/g, " "),
  );

  return withoutBlockComments
    .split("\n")
    .map((line) => {
      let quote = null;

      for (let i = 0; i < line.length - 1; i += 1) {
        const char = line[i];

        if (quote) {
          if (char === "\\") {
            i += 1;
          } else if (char === quote) {
            quote = null;
          }

          continue;
        }

        if (char === '"' || char === "'" || char === "`") {
          quote = char;

          continue;
        }

        if (char === "/" && line[i + 1] === "/") {
          return line.slice(0, i);
        }
      }

      return line;
    })
    .join("\n");
}

/** Every `invoke("<name>")` call in a source file. */
function readInvocations(source) {
  const calls = [];
  const pattern = /\binvoke\b/g;
  let match;

  while ((match = pattern.exec(source)) !== null) {
    let cursor = match.index + "invoke".length;

    const skipWhitespace = () => {
      while (cursor < source.length && /\s/.test(source[cursor])) {
        cursor += 1;
      }
    };

    skipWhitespace();

    // Skip an optional type argument, balancing angle brackets so
    // `invoke<Record<string, number>>` is handled.
    if (source[cursor] === "<") {
      let depth = 0;

      while (cursor < source.length) {
        if (source[cursor] === "<") {
          depth += 1;
        } else if (source[cursor] === ">") {
          depth -= 1;

          if (depth === 0) {
            cursor += 1;

            break;
          }
        }

        cursor += 1;
      }

      skipWhitespace();
    }

    if (source[cursor] !== "(") {
      continue;
    }

    cursor += 1;
    skipWhitespace();

    const quote = source[cursor];
    const terminating = { '"': /"/, "'": /'/, "`": /`/ }[quote];

    if (!terminating) {
      continue;
    }

    cursor += 1;

    const nameStart = cursor;

    while (cursor < source.length && !terminating.test(source[cursor])) {
      cursor += 1;
    }

    const name = source.slice(nameStart, cursor);

    if (!/^[a-z][a-z0-9_]*$/.test(name)) {
      continue;
    }

    const line = source.slice(0, match.index).split("\n").length;

    calls.push({ name, line });
  }

  return calls;
}

const registered = readRegisteredCommands(readSource(registryPath));
const invoked = [];

for (const path of collectSources(frontendRoot)) {
  for (const call of readInvocations(stripComments(readSource(path)))) {
    invoked.push({ ...call, file: relative(repoRoot, path).split(sep).join("/") });
  }
}

const missing = invoked.filter((call) => !registered.has(call.name));
const invokedNames = new Set(invoked.map((call) => call.name));
const uninvoked = [...registered].filter((name) => !invokedNames.has(name));

console.log(
  `IPC contract: ${invoked.length} invoke call sites, ${registered.size} registered commands.`,
);

if (missing.length > 0) {
  console.error("\nInvoked but NOT registered in generate_handler!:\n");

  for (const call of missing) {
    console.error(`  ${call.name}  (${call.file}:${call.line})`);
  }

  console.error(
    "\nEach of these fails at runtime with \"command not found\". Register the command in crates/desktop/src/lib.rs or fix the name at the call site.",
  );
}

if (uninvoked.length > 0) {
  console.log(
    `\nRegistered but never invoked from the webview (informational): ${uninvoked.sort().join(", ")}`,
  );
}

process.exit(missing.length > 0 ? 1 : 0);
