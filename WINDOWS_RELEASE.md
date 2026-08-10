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

Windows code signing is explicitly not a v1 release requirement. The expected
Microsoft SmartScreen / unknown-publisher warning is documented and accepted;
it is not a test failure. The workflow records the Authenticode status but does
not require or configure a certificate.

For a conventional CA certificate instead, Tauri supports a SHA-256
`certificateThumbprint` for a certificate installed in the Windows certificate
store. A password-protected PKCS#12/PFX can be imported into the runner from
encrypted GitHub secrets. Do not commit either the PFX or its password.

Signing can be added in a future release without changing the NSIS format. No
signing service, certificate, or signing secret is configured for v1.

## Release gates

After an installer is produced, install it on a clean Windows machine and
run the complete packaged PDF corpus and error/licensing matrix. Only a signed
installer that passes that matrix may be uploaded to a draft/prerelease asset
and used as `WINDOWS_DOWNLOAD_URL`. Record the accepted unsigned-publisher
warning. Do not create final `v1.0.0` from this workflow.
