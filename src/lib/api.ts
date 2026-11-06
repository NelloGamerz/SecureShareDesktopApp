import axios, { AxiosError, type InternalAxiosRequestConfig } from "axios";
import { toast } from "sonner";
import { env } from "@/lib/env";
import { error } from "@tauri-apps/plugin-log";
import { getDeviceIdentifier } from "@/services/getDeviceInfo";

type ApiPayload = { message?: unknown; success?: unknown };

function successMessage(data: unknown): string {
  const message = (data as ApiPayload | null)?.message;
  return typeof message === "string" && message.trim()
    ? message
    : "Request completed successfully.";
}

// A 2xx can still be a business failure (`{ success: false }`); the page owns
// that message, so do not report it as a success.
function isBusinessFailure(data: unknown): boolean {
  return (data as ApiPayload | null)?.success === false;
}

// Prefer the backend's own wording over axios' "Request failed with status…".
function responseMessage(data: unknown): string | undefined {
  if (!data || typeof data !== "object") return undefined;

  const { message, error } = data as ApiPayload & { error?: unknown };

  if (typeof message === "string" && message.trim()) return message;
  if (typeof error === "string" && error.trim()) return error;
  if (error && typeof error === "object") {
    const nested = (error as { message?: unknown }).message;
    if (typeof nested === "string" && nested.trim()) return nested;
  }

  return undefined;
}

const api = axios.create({
  baseURL: env.apiBaseUrl,
  timeout: 30_000,
  headers: {
    "Content-Type": "application/json",
  },
});

let tokenGetter: (() => Promise<string | null>) | null = null;

/**
 * Called once from `AxiosProvider` to wire the desktop session token into
 * axios. The getter reads from the Rust `AuthState`, so it always returns the
 * current token and refreshes it when it is near expiry.
 */
export function setTokenGetter(getter: () => Promise<string | null>) {
  tokenGetter = getter;
}

api.interceptors.request.use(async (config: InternalAxiosRequestConfig) => {
  if (tokenGetter) {
    try {
      const token = await tokenGetter();

      if (token) {
        config.headers.Authorization = `Bearer ${token}`;
      }

      const deviceIdentifier = await getDeviceIdentifier();

      config.headers["X-Device-Id"] = deviceIdentifier;
    } catch (err) {
      // Log the failure only. Never log the token or the header value.
      await error(`Failed to attach the authorization header: ${String(err)}`);
    }
  }

  return config;
});

api.interceptors.response.use(
  (response) => {
    const method = response.config.method?.toLowerCase();

    // Reads are not user actions; toasting them would fire on every poll.
    if (
      method &&
      method !== "get" &&
      method !== "head" &&
      !isBusinessFailure(response.data)
    ) {
      toast.success(successMessage(response.data));
    }

    return response;
  },
  (error: AxiosError) => {
    const status = error.response?.status;
    const message =
      responseMessage(error.response?.data) ??
      error.message ??
      "Unexpected error";

    // One id so a repeated failure (e.g. polling) updates instead of stacking.
    if (status === 401) {
      toast.error(responseMessage(error.response?.data) ?? "Your session has expired. Please sign in again.", {
        id: "api-error",
      });

      return Promise.reject(
        Object.assign(
          new Error("Your session has expired. Please sign in again."),
          {
            status,
            isAuthError: true,
          },
        ),
      );
    }

    toast.error(message, { id: "api-error" });

    return Promise.reject(
      Object.assign(new Error(message), { status, isAxiosError: true }),
    );
  },
);

export { api };
export default api;
