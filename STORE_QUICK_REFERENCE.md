# Microsoft Store Implementation - Quick Summary

## Changes Made

### ✅ Files Created
1. **`crates/desktop/tauri.windows.store.conf.json`** - Store-specific Tauri config with updater disabled

### ✅ Files Modified
1. **`crates/desktop/Cargo.toml`** - Added `enable-updater` feature flag
2. **`crates/desktop/src/lib.rs`** - Wrapped updater code with `#[cfg(feature = "enable-updater")]`
3. **`.github/workflows/release.yml`** - Added `build-windows-store` and `publish-windows-store` jobs

### ✅ Documentation Created
1. **`MICROSOFT_STORE_IMPLEMENTATION.md`** - Comprehensive guide (this file covers detailed setup)

---

## Architecture Summary

```
EXISTING (✓ UNCHANGED):
├─ Linux → native installer → GitHub Release + R2 → Tauri updater
├─ macOS → native installer → GitHub Release + R2 → Tauri updater  
└─ Windows → EXE/MSI → GitHub Release + R2 → Tauri updater

NEW (✓ ADDITIVE):
└─ Windows Store → MSIX → Microsoft Store → Microsoft Store updates
```

---

## Key Differences: Direct Windows vs Store

| Aspect | Direct (EXE/MSI) | Microsoft Store (MSIX) |
|--------|------------------|------------------------|
| Updater | Tauri (enabled) | None (disabled) |
| Distribution | GitHub Releases + R2 | Microsoft Store |
| Updates | Check `latest.json` | Store automatic |
| Config | `tauri.windows.conf.json` | `tauri.windows.store.conf.json` |
| Build Flag | Default (updater enabled) | `--no-default-features` |

---

## How to Release

### Simple: Create a tag

```bash
git tag v0.3.0
git push origin v0.3.0
```

The workflow automatically:
1. Builds Linux, Windows, macOS (existing)
2. Builds Windows Store MSIX (new)
3. Uploads direct installers to GitHub Release + R2
4. Stores MSIX as GitHub artifact (ready for Store)

---

## Critical: Things That Did NOT Change

- ✅ Linux builds work exactly as before
- ✅ macOS builds work exactly as before
- ✅ Windows EXE/MSI builds work exactly as before
- ✅ Tauri updater for direct users works exactly as before
- ✅ R2 distribution unchanged
- ✅ latest.json unchanged
- ✅ GitHub Release format unchanged

---

## When You're Ready for Microsoft Store

1. Create Partner Center account (~$99 fee)
2. Set up app identity
3. Create Azure AD app
4. Add 3 GitHub secrets:
   - `MS_STORE_TENANT_ID`
   - `MS_STORE_CLIENT_ID`
   - `MS_STORE_CLIENT_SECRET`
5. Enable `publish-windows-store` job in workflow
6. Next release automatically publishes to Store

Until then, the MSIX is built but stored as GitHub artifact - safe for testing.

---

## Testing MSIX Locally

```powershell
# 1. Download MSIX from GitHub Actions artifact
# 2. Enable Developer Mode: Settings → For developers → Developer Mode
# 3. Install:
Add-AppxPackage -Path "C:\path\to\VilSend_0.3.0.0_x64_Store.msix"

# 4. Launch from Start Menu
# 5. Verify cloudflared starts
# 6. Uninstall:
Get-AppxPackage | Where-Object { $_.Name -like "*VilSend*" } | Remove-AppxPackage
```

---

## Technical Details

### Feature Flag System

- **Default build**: `enable-updater` feature is ON
  - Updater code compiled in
  - Updater runs at startup

- **Store build**: `enable-updater` feature is OFF (`--no-default-features`)
  - Updater code NOT compiled in
  - Updater disabled entirely

### Resource Bundling

- **cloudflared.exe** included in MSIX
- Located at: `resources/cloudflared/windows-x64/cloudflared.exe`
- Accessible via: `app.path().resource_dir()`
- Works in both dev and packaged environments

### Version Conversion

- Git tag: `v0.3.0`
- Tauri version: `0.3.0`
- MSIX version: `0.3.0.0` (automatic conversion)

---

## GitHub Actions Secrets - No New Ones Required Yet

Current workflow uses existing secrets:
- `TAURI_SIGNING_PRIVATE_KEY`
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`
- `VITE_CLERK_PUBLISHABLE_KEY`
- `VITE_API_BASE_URL`
- `VITE_APP_NAME`
- `GITHUB_TOKEN`
- `R2_*` (for updater upload)

**New secrets only needed when enabling Partner Center publishing**.

---

## Verification Checklist

- [x] Store config created with updater disabled
- [x] Feature flag added to Cargo.toml
- [x] Updater imports conditional in lib.rs
- [x] Updater initialization conditional in lib.rs
- [x] GitHub Actions build job added
- [x] GitHub Actions publish job added (disabled)
- [x] MSIX build uses `--no-default-features`
- [x] MSIX stored as artifact (not R2)
- [x] Existing release jobs unchanged
- [x] Existing upload-updater job unchanged
- [x] Existing Linux/macOS configs unchanged
- [x] cloudflared.exe included in MSIX config

---

## Troubleshooting

### "Build fails: --no-default-features not recognized"

Your Tauri CLI might be older. Update:
```bash
npm install -g @tauri-apps/cli@latest
```

### "MSIX version error"

MSIX requires `MAJOR.MINOR.PATCH.BUILD` format.  
Workflow converts `0.3.0` → `0.3.0.0` automatically.

### "Updater still runs in Store build"

Check:
1. Cargo.toml has `[features]` section
2. lib.rs has `#[cfg(feature = "enable-updater")]`
3. Workflow uses `--no-default-features`
4. Clean cache: `cargo clean`

---

## Next Steps

1. **Test build**: Tag as `v0.2.10-test` and monitor GitHub Actions
2. **Test MSIX**: Download artifact and install locally
3. **When ready**: Set up Partner Center and enable publishing job
4. **Submit**: Use Partner Center UI to submit MSIX to Microsoft Store

---

See `MICROSOFT_STORE_IMPLEMENTATION.md` for complete detailed guide.
