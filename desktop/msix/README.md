# Beam in the Microsoft Store

The Store signs the package, so people install Beam with one click and never
see the SmartScreen "Windows protected your PC" warning that the unsigned
`.msi` from GitHub gets.

## One-time setup in Partner Center

1. Sign in at [partner.microsoft.com/dashboard](https://partner.microsoft.com/dashboard)
   with the developer account.
2. **Apps and games → New product → MSIX or PWA app**, and reserve the name
   **Beam** (or the closest free name).
3. Open the product, then **Product management → Product identity**. Copy:
   - `Package/Identity/Name`, e.g. `12345Kartikay.Beam`;
   - `Package/Identity/Publisher`, e.g. `CN=1A2B3C4D-…`;
   - `Package/Properties/PublisherDisplayName`.

## Build the package

From `desktop/`, after a release build:

```powershell
npm run tauri build -- --bundles msi
./msix/build-msix.ps1 -IdentityName "<Name>" -Publisher "<Publisher>" -PublisherDisplayName "<Display name>" -TargetDir D:/beam-target
```

This writes `msix/Beam_<version>.0_x64.msix` under the target directory.
CI builds the same package with placeholder identity values, to check it
still packs.

## Submit

**Start your submission**, then fill in:

- **Packages:** upload the `.msix`.
- **Properties:** category *Productivity* (or *Utilities & tools*).
- **Restricted capabilities.** The package asks for two, and the Store asks
  why. Suggested answers:
  - `runFullTrust`: "Beam is a desktop (Win32) app built with Tauri."
  - `unvirtualizedResources`: "Beam adds a 'Send to → Beam' shortcut in the
    user's SendTo folder and an optional start-with-Windows entry. Both must
    be visible to Explorer and Windows, so writes to AppData and HKCU can't
    be virtualized."
- **Store listing:** description, screenshots (at least one, 1366×768 or
  larger), and the privacy policy URL:
  `https://github.com/kartikay633/Beam/blob/main/docs/PRIVACY.md`.
- **Age ratings:** the questionnaire (no user-generated content shared
  publicly; files go device to device).

Certification usually takes one to three business days.

## Testing the package locally

A Store-identity package installs only once it's signed. To try it on your
own PC first:

1. Turn on **Settings → System → For developers → Developer Mode**.
2. Create a test certificate whose subject matches `-Publisher`, and sign:

   ```powershell
   $cert = New-SelfSignedCertificate -Type Custom -Subject "CN=Beam Dev" -KeyUsage DigitalSignature `
     -FriendlyName "Beam test" -CertStoreLocation Cert:\CurrentUser\My `
     -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
   & "C:\Program Files (x86)\Windows Kits\10\bin\10.0.22621.0\x64\signtool.exe" sign /fd SHA256 /sha1 $cert.Thumbprint <package.msix>
   ```

3. Trust the certificate (export it, import into *Trusted People*), then
   double-click the `.msix`.
