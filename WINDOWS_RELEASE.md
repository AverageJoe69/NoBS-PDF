# Windows release preparation

macOS release work is paused pending Apple Developer Program membership. It is
not part of this Windows release path.

## Supported production target

- Architecture: `x86_64-pc-windows-msvc` (Windows x64).
- Installer: Tauri NSIS `-setup.exe`.
- WebView: Microsoft Edge WebView2 using Tauri's downloaded bootstrapper when
  the evergreen runtime is absent. Windows 10 and 11 normally include it.
- Native PDF renderer: PDFium revision 7350, matching `pdfium-render = 0.8.35`
  and its `pdfium_7350` feature.
- PDFium source archive:
  `https://github.com/bblanchon/pdfium-binaries/releases/download/chromium%2F7350/pdfium-win-x64.tgz`
- Archive SHA-256:
  `0289ee5f6beaa001c1fa0fc96dfdbc2f0238cd4b1cfc72a75ac473fa191dfbe6`.
- `pdfium.dll` SHA-256:
  `8f9600aeeb4f0ed4d5ab55ba772903c04175b05f13137f2712efe886d8c5b60b`.

The DLL is downloaded and checksum-verified by the Windows RC workflow, then
bundled as `pdfium.dll`. Do not silently substitute a newer PDFium build.

## Build requirements

Run the release build on a native Windows x64 runner with the MSVC Rust target,
Visual Studio C++/Windows SDK tools, Node.js, npm, and WebView2 available. Tauri
supports macOS-to-Windows NSIS cross-compilation with caveats, but recommends a
Windows machine or CI when available. Packaged testing must also run on Windows.

The application uses Windows Credential Manager through keyring's
`windows-native` backend for its durable activation token. PDF processing uses
only the local licence gate and never sends document data to the licensing API.

## Production signing

Unsigned installers must never be published. The prepared GitHub workflow uses
Microsoft Azure Artifact Signing (formerly Trusted Signing) Public Trust and
OIDC. It signs the application executable before bundling, signs the completed
NSIS installer, verifies both Authenticode signatures, and only then uploads a
private workflow artifact.

Required Azure/GitHub configuration:

1. An Azure Artifact Signing account with verified identity and a Public Trust
   certificate profile.
2. An Entra application/service principal assigned the **Artifact Signing
   Certificate Profile Signer** role on that profile.
3. An OIDC federated credential restricted to this repository and the chosen
   GitHub environment/ref.
4. GitHub Actions secrets: `AZURE_CLIENT_ID`, `AZURE_TENANT_ID`, and
   `AZURE_SUBSCRIPTION_ID`.
5. GitHub Actions variables: `AZURE_ARTIFACT_SIGNING_ENDPOINT`,
   `AZURE_ARTIFACT_SIGNING_ACCOUNT`, and
   `AZURE_ARTIFACT_SIGNING_CERTIFICATE_PROFILE`.

For a conventional CA certificate instead, Tauri supports a SHA-256
`certificateThumbprint` for a certificate installed in the Windows certificate
store. A password-protected PKCS#12/PFX can be imported into the runner from
encrypted GitHub secrets. Do not commit either the PFX or its password.

EV is not required merely for SmartScreen: Microsoft now applies the same
reputation-building model to EV, OV, and Artifact Signing. For a UK
organization, Azure Artifact Signing Public Trust is the preferred managed
option. A UK individual is not currently eligible for Public Trust and should
use an OV certificate from a trusted CA or the Microsoft Store route.

## Release gates

After a signed installer is produced, install it on a clean Windows machine and
run the complete packaged PDF corpus and error/licensing matrix. Only a signed
installer that passes that matrix may be uploaded to a draft/prerelease asset
and used as `WINDOWS_DOWNLOAD_URL`. Do not create final `v1.0.0` from this
workflow.
