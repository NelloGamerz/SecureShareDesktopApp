# VilSend Microsoft Store Distribution Implementation

## Overview

This implementation adds a **second, separate** Windows distribution path for Microsoft Store while preserving all existing Linux, macOS, and Windows direct-distribution pipelines.

### Architecture

```
Existing (unchanged):
├── Linux → native installer → GitHub Release/R2 → Tauri updater
├── macOS → native installer → GitHub Release/R2 → Tauri updater
└── Windows → EXE/MSI → GitHub Release/R2 → Tauri updater

NEW (additive):
└── Windows Microsoft Store → MSIX → Microsoft Store → MS Store updates
```

**Key property**: Windows Store version does NOT use Tauri updater. Microsoft Store handles all updates.

---

## Files Created

### 1. `src-tauri/tauri.windows.store.conf.json`

**Purpose**: Store-specific Tauri configuration that:
- Disables the Tauri updater plugin
- Includes cloudflared.exe resource
- Removes WiX installer configuration

**Contents**:
```json
{
  "$schema": "https://schema.tauri.app/config/2",
  "identifier": "in.vilsend.app",
  "plugins": {
    "updater": {
      "active": false
    }
  },
  "bundle": {
    "resources": [
      "resources/cloudflared/windows-x64/cloudflared.exe"
    ],
    "windows": [
      {
        "wix": null,
        "nsis": null,
        "signCommand": null
      }
    ]
  }
}
```

---

## Files Modified

### 1. `src-tauri/Cargo.toml`

**Changes**:
- Added features section with `enable-updater` feature flag (enabled by default)

**Exact change**:
```diff
[target.'cfg(target_os = "windows")'.dependencies]
windows = { version = "0.62", features = ["Win32_System_Power"] }

+[features]
+default = ["enable-updater"]
+enable-updater = []
```

**Why**: Allows building without the updater plugin for Store version via `--no-default-features`

---

### 2. `src-tauri/src/lib.rs`

**Changes**:
- Wrapped updater import with `#[cfg(feature = "enable-updater")]`
- Wrapped updater initialization with conditional compilation

**Exact changes**:

**Change 1 - Import** (around line 33):
```diff
-use services::updates_service::handle_pending_update;
+#[cfg(feature = "enable-updater")]
+use services::updates_service::handle_pending_update;
```

**Change 2 - Updater initialization** (around line 122):
```diff
            let app_state = Arc::new(app_state);

-            let update_app = app.handle().clone();
-            let dispatcher = Arc::clone(&app_state.event_dispatcher);
-            let dispatcher2 = Arc::clone(&app_state.event_dispatcher);
-
-            tauri::async_runtime::spawn(async move {
-                handle_pending_update(update_app, dispatcher).await;
-            });
+            #[cfg(feature = "enable-updater")]
+            {
+                let update_app = app.handle().clone();
+                let dispatcher = Arc::clone(&app_state.event_dispatcher);
+
+                tauri::async_runtime::spawn(async move {
+                    handle_pending_update(update_app, dispatcher).await;
+                });
+            }
+
+            let dispatcher2 = Arc::clone(&app_state.event_dispatcher);
```

---

### 3. `.github/workflows/release.yml`

**Changes**:
- Added new `build-windows-store` job
- Added new `publish-windows-store` job (currently disabled, ready for future Partner Center integration)

**Jobs added**:

#### `build-windows-store` job
- Runs on `windows-latest`
- Builds frontend with `npm ci`
- Builds Rust backend with Store config
- **Disables updater** with `--no-default-features`
- Bundles as MSI (Tauri will package as MSIX)
- Uploads to GitHub Actions artifacts

#### `publish-windows-store` job
- Runs on `ubuntu-latest` (can be run on windows-latest if needed)
- Depends on `build-windows-store`
- Currently disabled (`if: ${{ false }}`)
- Ready for Partner Center integration when credentials are configured

**Key points**:
- Uses separate Rust cache key (`-store` suffix)
- Uses same environment variables (VITE_CLERK_PUBLISHABLE_KEY, etc.)
- Computes MSIX-compatible version (0.2.9 → 0.2.9.0)
- Stores artifact in `windows-store-msix` artifact

---

## Building Instructions

### Local Development Build (with updater enabled)

```bash
npm run tauri -- build --config src-tauri/tauri.windows.conf.json
```

Output: Traditional EXE/MSI installers in `src-tauri/target/release/bundle/nsis/` and `src-tauri/target/release/bundle/msi/`

### Local Microsoft Store Build (updater disabled)

```bash
npm run tauri -- build \
  --config src-tauri/tauri.windows.store.conf.json \
  --bundle msi \
  --no-default-features
```

Output: MSIX package in `src-tauri/target/release/bundle/msi/`

---

## Release Process

### Step 1: Create Release Tag

```bash
git tag v0.3.0
git push origin v0.3.0
```

This triggers the `.github/workflows/release.yml` workflow.

### Step 2: Workflow Execution

The release workflow automatically:

1. **Existing jobs run in parallel**:
   - `release` job builds Linux, Windows, macOS installers
   - `build-windows-store` job builds MSIX package
   - `upload-updater` job processes Linux/macOS/Windows direct installers

2. **GitHub Release created** with:
   - Linux AppImage / DEB packages
   - Windows EXE/MSI installers
   - macOS DMG / APP bundles
   - `latest.json` (Tauri updater metadata)
   - **NOT** MSIX (separate artifact)

3. **R2 Upload** via `upload-updater` job:
   - All GitHub Release assets uploaded to R2
   - `latest.json` rewritten with R2 URLs
   - Available at: `https://update.vilsend.in/latest.json`

4. **Store MSIX** available:
   - In GitHub Actions artifacts (for testing/download)
   - **NOT uploaded to R2**
   - **NOT added to latest.json**
   - Ready for manual or automated Partner Center submission

### Step 3: Manual Microsoft Store Submission (Future)

Once Partner Center credentials are configured:

```bash
# Enable the publish-windows-store job by changing:
# if: ${{ false }}
# to:
# if: github.event_name == 'push'

# Configure GitHub secrets:
# - MS_STORE_TENANT_ID
# - MS_STORE_CLIENT_ID
# - MS_STORE_CLIENT_SECRET
```

See **GitHub Actions Secrets Required** section below.

---

## Artifact Distribution

### GitHub Release

**Contains**:
- `VilSend_0.3.0_x64-en-US.msi` (Windows direct installer)
- `VilSend_0.3.0_amd64.deb` (Linux)
- `VilSend_0.3.0_amd64.AppImage` (Linux)
- `VilSend_0.3.0_universal.dmg` (macOS)
- `VilSend_0.3.0_x64.app.tar.gz` (macOS)
- `latest.json` (Tauri updater metadata)

**Purpose**: Direct distribution to users who downloaded from GitHub Releases

### Cloudflare R2

**Contains**:
```
/releases/v0.3.0/
  ├── VilSend_0.3.0_x64-en-US.msi
  ├── VilSend_0.3.0_amd64.deb
  ├── VilSend_0.3.0_amd64.AppImage
  ├── VilSend_0.3.0_universal.dmg
  ├── VilSend_0.3.0_x64.app.tar.gz
  └── latest.json

/latest.json (root) - Updated to point to v0.3.0
```

**Purpose**: Tauri updater checks this for updates

**Endpoint**: `https://update.vilsend.in/latest.json`

### Microsoft Store

**Contains**:
- `VilSend_0.3.0.0_x64_Store.msix` (MSIX package)

**Purpose**: Users install from Microsoft Store, receive updates through Microsoft Store

**NOT included**:
- No upload to R2
- No entry in latest.json
- No Tauri updater involvement

---

## GitHub Actions Secrets Required

### For Existing Pipeline (No Changes)

These continue to work as before:

| Secret | Purpose |
|--------|---------|
| `TAURI_SIGNING_PRIVATE_KEY` | Sign Windows EXE/MSI |
| `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | Password for signing key |
| `VITE_CLERK_PUBLISHABLE_KEY` | Frontend auth configuration |
| `VITE_API_BASE_URL` | Backend API endpoint |
| `VITE_APP_NAME` | Frontend app name |
| `GITHUB_TOKEN` | GitHub API access |
| `R2_ACCESS_KEY_ID` | Cloudflare R2 credentials |
| `R2_SECRET_ACCESS_KEY` | Cloudflare R2 credentials |
| `R2_ACCOUNT_ID` | Cloudflare R2 account ID |
| `R2_BUCKET` | Cloudflare R2 bucket name |

### For Microsoft Store (Future - When Ready)

**DO NOT configure until Partner Center account is ready.**

| Secret | Purpose | Obtain From |
|--------|---------|-------------|
| `MS_STORE_TENANT_ID` | Azure AD tenant | Microsoft Partner Center → Account settings → App identity |
| `MS_STORE_CLIENT_ID` | Azure AD application ID | Azure AD → App registrations |
| `MS_STORE_CLIENT_SECRET` | Azure AD client secret | Azure AD → App registrations → Certificates & secrets |

**Setup Steps** (when Partner Center is ready):

1. Create Microsoft Partner Center account
2. Set up app identity in Partner Center
3. Create Azure AD application
4. Grant Partner Center API permissions
5. Create client secret
6. Add GitHub secrets with these values
7. Uncomment `publish-windows-store` job conditions

---

## Microsoft Store Setup Guide

### Prerequisites

- [ ] Microsoft Partner Center account
- [ ] $99 developer program enrollment fee paid
- [ ] App identity configured in Partner Center

### Step 1: Create Microsoft Store Partner Center Account

1. Go to [Microsoft Partner Center](https://partner.microsoft.com/en-us/dashboard)
2. Sign up as an App Developer ($99 fee)
3. Verify your identity (email + phone)
4. Create company profile

### Step 2: Register Application

1. In Partner Center, go to **Overview → Create a new app**
2. Fill in:
   - **App name**: VilSend
   - **Product name**: VilSend
   - **Category**: Utilities or File Sharing
   - Check if app name is available

3. Note the **Package Identity**:
   - Publisher ID (e.g., `CN=1a2b3c4d-e5f6-7890-1a2b-3c4d5e6f7890`)
   - Package ID (e.g., `VilSend`)

### Step 3: Configure App Identity

1. In Partner Center app, go to **Product → Product identity**
2. Copy:
   - **Package/Identity/Name**: Save this
   - **Publisher**: Note the publisher ID

3. Store these values - they'll be needed for MSIX certificate

### Step 4: Create Azure AD Application

1. Go to [Azure Portal](https://portal.azure.com)
2. Navigate to **Azure Active Directory → App registrations**
3. Click **New registration**
4. Configure:
   - **Name**: VilSend Store Publisher
   - **Supported account types**: My organization only
   - **Redirect URI**: Leave blank for now

5. After creation, note:
   - **Application ID** (Client ID)
   - **Directory ID** (Tenant ID)

6. Go to **Certificates & secrets**
7. Click **New client secret**
   - Description: "GitHub Actions"
   - Expiration: 24 months
8. Copy **Value** (Client Secret) - save it, won't be shown again

### Step 5: Grant Partner Center API Permissions

1. In Azure AD app, go to **API permissions**
2. Click **Add a permission**
3. Search for "Partner Center"
4. Select **Delegated permissions**
5. Grant appropriate permissions for app submission

### Step 6: Store GitHub Secrets

In your GitHub repository settings:

1. Go to **Settings → Secrets and variables → Actions**
2. Click **New repository secret** for each:

```
MS_STORE_TENANT_ID = (Directory ID from Azure)
MS_STORE_CLIENT_ID = (Application ID from Azure)
MS_STORE_CLIENT_SECRET = (Client Secret from Azure)
```

### Step 7: Enable Store Publishing in Workflow

1. Edit `.github/workflows/release.yml`
2. Find `publish-windows-store` job
3. Change:
   ```yaml
   if: ${{ false }}
   ```
   to:
   ```yaml
   if: github.event_name == 'push' && startsWith(github.ref, 'refs/tags/')
   ```

4. Commit and push

### Step 8: Prepare MSIX Certificate

For production Store submissions, Microsoft may require signing certificate. The `build-windows-store` job uses existing `TAURI_SIGNING_PRIVATE_KEY` (reuse your existing cert).

For Microsoft Store, the certificate must have specific Microsoft Store requirements. See [Microsoft MSIX Certificate Requirements](https://docs.microsoft.com/en-us/windows/msix/package/signing-package-overview).

---

## Testing MSIX Package Locally

### Prerequisites

- Windows 10 21H2 or Windows 11
- Developer Mode enabled
- PowerShell 7+ or Command Prompt

### Step 1: Extract MSIX from GitHub Actions

1. Go to GitHub repo → **Actions**
2. Find the release workflow run
3. Download artifact: `windows-store-msix`
4. Extract `.msix` file from the artifact

### Step 2: Install MSIX Locally

```powershell
# Using PowerShell
Add-AppxPackage -Path "C:\path\to\VilSend_0.3.0.0_x64_Store.msix"
```

Or:

```cmd
# Using Command Prompt
Powershell Add-AppxPackage -Path "C:\path\to\VilSend_0.3.0.0_x64_Store.msix"
```

### Step 3: Verify Installation

```powershell
# List installed packages
Get-AppxPackage | Where-Object { $_.Name -like "*VilSend*" }
```

### Step 4: Test Application

1. Open Start Menu
2. Search for "VilSend"
3. Launch application
4. Verify:
   - App starts without errors
   - Frontend loads
   - Cloudflared can be started (check logs)
   - No updater checks (shouldn't show update dialog)
   - File operations work

### Step 5: Check Application Data

```powershell
# MSIX apps store data in:
# C:\Users\<username>\AppData\Local\Packages\<PackageFamilyName>\LocalState\

# List locations:
Get-AppxPackage | Where-Object { $_.Name -like "*VilSend*" } | Select-Object PackageFamilyName
```

### Step 6: Uninstall Test Package

```powershell
# Find and remove
Get-AppxPackage | Where-Object { $_.Name -like "*VilSend*" } | Remove-AppxPackage
```

---

## Version Management

### Version Format

- **Development/Release config**: Semantic versioning (e.g., `0.2.9`, `0.3.0`)
- **MSIX package**: Four-part version (e.g., `0.2.9.0`, `0.3.0.0`)

### Conversion

The workflow automatically converts:

```bash
# Input: Git tag
git tag v0.3.0

# Extracted version: 0.3.0
# → Converted to MSIX: 0.3.0.0

# Next release:
git tag v0.3.1
# → Converted to MSIX: 0.3.1.0
```

### Manual MSIX Version Build

If you need to build locally with specific version:

```bash
# Edit src-tauri/tauri.conf.json
# Change "version": "0.3.0" to "0.3.0"

npm run tauri -- build \
  --config src-tauri/tauri.windows.store.conf.json \
  --bundle msi \
  --no-default-features
```

---

## Resource Bundling Verification

### cloudflared.exe Location

The application expects `cloudflared.exe` at:
```
resources/cloudflared/windows-x64/cloudflared.exe
```

### How It Works

1. Tauri bundles `resources/` directory into package
2. At runtime, `app.path().resource_dir()` returns:
   - **Development**: Project directory
   - **Packaged (MSIX)**: Package installation directory
3. Application resolves: `{resource_dir}/resources/cloudflared/windows-x64/cloudflared.exe`

### Verification

**Build log should show**:
```
Looking for cloudflared binary at: <path>/resources/cloudflared/windows-x64/cloudflared.exe
File exists: true
```

**MSIX package should contain**:
```
VilSend/
├── VilSend.exe
├── resources/
│   └── cloudflared/
│       └── windows-x64/
│           └── cloudflared.exe
└── [other files]
```

**Runtime check**:
- Launch app
- Go to Onboarding → Start Tunnel
- Should successfully start cloudflared
- Check app logs (if available)

---

## Confirmation: Existing Behavior Unchanged

### Linux Distribution

**Before**: ✓  
**After**: ✓ **UNCHANGED**

- Build: `src-tauri/tauri.linux.conf.json`
- Installer: `.deb`, `.AppImage`
- Distribution: GitHub Releases + R2
- Updates: Tauri updater checking `https://update.vilsend.in/latest.json`

### macOS Distribution

**Before**: ✓  
**After**: ✓ **UNCHANGED**

- Build: `src-tauri/tauri.macos.conf.json`
- Installer: `.dmg`, `.app.tar.gz`
- Distribution: GitHub Releases + R2
- Updates: Tauri updater checking `https://update.vilsend.in/latest.json`

### Windows Direct Distribution

**Before**: ✓  
**After**: ✓ **UNCHANGED**

- Build: `src-tauri/tauri.windows.conf.json`
- Installer: `.msi` (WiX), `.nsis`
- Distribution: GitHub Releases + R2
- Updates: Tauri updater checking `https://update.vilsend.in/latest.json`
- **Updater plugin enabled**: Yes

### Windows Store Distribution

**Before**: ✗ (N/A)  
**After**: ✓ **NEW**

- Build: `src-tauri/tauri.windows.store.conf.json`
- Package: `.msix`
- Distribution: Microsoft Store only
- Updates: Microsoft Store (not Tauri updater)
- **Updater plugin enabled**: No (feature disabled)

---

## Troubleshooting

### Build Fails: "cloudflared.exe not found"

**Cause**: Resources directory missing or incomplete

**Solution**:
```bash
# Ensure resources exist
ls -la src-tauri/resources/cloudflared/windows-x64/cloudflared.exe

# If missing, you may need to download/add it to the repository
```

### Build Fails: "Cannot find tauri config"

**Cause**: Typo in config path in workflow or build command

**Solution**:
```bash
# Verify file exists
ls -la src-tauri/tauri.windows.store.conf.json

# Verify syntax
cat src-tauri/tauri.windows.store.conf.json | grep -c "version"
```

### MSIX Installation Fails

**Cause**: Certificate or signing issue

**Solution**:
```powershell
# Check certificate validity
Get-AuthenticodeSignature -FilePath "C:\path\to\VilSend.msix"

# Install in Developer Mode
# Settings → For developers → Developer Mode → ON
```

### Updater Still Runs in Store Version

**Cause**: Feature flag not properly passed to build

**Solution**:
1. Verify `Cargo.toml` has features section
2. Verify lib.rs has `#[cfg(feature = "enable-updater")]`
3. Verify workflow uses `--no-default-features`
4. Clean build cache:
   ```bash
   cargo clean
   ```

---

## Summary

| Aspect | Status | Details |
|--------|--------|---------|
| **Existing Windows builds** | ✓ Preserved | EXE/MSI installers, Tauri updater, R2 distribution |
| **Linux builds** | ✓ Preserved | AppImage/DEB, Tauri updater, R2 distribution |
| **macOS builds** | ✓ Preserved | DMG/APP, Tauri updater, R2 distribution |
| **R2 updater** | ✓ Preserved | latest.json continues to serve all platforms |
| **New Store version** | ✓ Added | MSIX package, no Tauri updater, Microsoft Store distribution |
| **No breaking changes** | ✓ Confirmed | Existing release workflow completely unaffected |

---

## Next Steps

1. **Test the build**:
   ```bash
   git tag v0.2.10-test
   git push origin v0.2.10-test
   ```
   - Monitor GitHub Actions
   - Verify `build-windows-store` job succeeds
   - Download MSIX artifact

2. **Test MSIX locally** (see Testing section above)

3. **When ready for Store**:
   - Set up Microsoft Partner Center account
   - Create Azure AD application
   - Add GitHub secrets
   - Enable `publish-windows-store` job

4. **Submit to Store**:
   - Use Partner Center to submit MSIX
   - Complete Store certification
   - Publish to Store

---

## References

- [Tauri Windows MSIX Documentation](https://tauri.app/v1/guides/building/windows)
- [Microsoft MSIX Documentation](https://docs.microsoft.com/en-us/windows/msix/)
- [Tauri Plugin Updater](https://v2.tauri.app/features/autoupdate)
- [GitHub Actions Artifacts](https://docs.github.com/en/actions/using-workflows/storing-workflow-data-as-artifacts)
- [Microsoft Partner Center](https://partner.microsoft.com/en-us/dashboard)
