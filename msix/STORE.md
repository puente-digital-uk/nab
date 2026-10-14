# Publishing Nab on the Microsoft Store

The Store signs the package itself, so a Store install never shows a SmartScreen warning and
uninstalls cleanly from Settings › Apps. Individual developer registration is free.

## 1. Register (once, ~15 minutes plus identity verification)

1. Go to https://partner.microsoft.com/dashboard and sign in with a Microsoft account (create one for Puente Digital if you'd rather keep it separate from personal).
2. Enrol in the **Windows & Xbox** developer programme. Choose **Individual** (free) unless Puente Digital is a registered company and you want "Company" (paid, needs business documents). Individual is fine for a free app; the publisher display name can still be "Puente Digital".
3. Complete identity verification when asked (ID document and a selfie; it can take a day or two).

## 2. Reserve the name and read off the identity (5 minutes)

1. **Apps and games → New product → MSIX or PWA app**, reserve the name **Nab**.
2. Open the product → **Product management → Product identity**. Copy these three values:
   - **Package/Identity/Name** — e.g. `12345PuenteDigital.Nab`
   - **Package/Identity/Publisher** — e.g. `CN=1A2B3C4D-...`
   - **Package/Properties/PublisherDisplayName** — e.g. `Puente Digital`
3. In the GitHub repo: **Settings → Secrets and variables → Actions → Variables**, add
   `STORE_IDENTITY_NAME`, `STORE_PUBLISHER` and `STORE_PUBLISHER_DISPLAY` with those values.
   From then on every CI run builds a `Nab.msix` with the right identity (download it from the run's artifacts).

## 3. Test the package locally before uploading (10 minutes)

```powershell
.\build.ps1                                  # tests + release exe
.\msix\build.ps1 -DevSign -IdentityName "<Name from step 2>" -Publisher "<Publisher from step 2>" -PublisherDisplay "Puente Digital"
.\msix\install-dev.ps1                       # admin prompt once, to trust the test certificate
```

Then check, from the installed copy (Start → Nab):
- Ctrl+Shift+click and Alt+Space work.
- Tick **Start with Windows** → it appears in Settings › Apps › Startup as "Nab". Sign out and in: Nab is in the tray and no window opened.
- Untick it → it disappears from Startup.
- Uninstall: `Get-AppxPackage *Nab* | Remove-AppxPackage`.

Run the Windows App Certification Kit too if you have the SDK (`appcert.exe test -appxpackage dist\Nab.msix -reportoutputpath wack.xml`); it catches the things certification would reject.

## 4. Submit (20 minutes the first time)

1. Partner Center → the product → **Start your submission**.
2. **Packages:** upload the **unsigned** `Nab.msix` from the CI artifact (built with the Store identity from step 2). Don't upload a dev-signed one.
3. **Properties:** category **Productivity**; privacy policy URL `https://github.com/puente-digital-uk/nab/blob/main/PRIVACY.md`; website `https://www.puentedigital.co.uk/`; support contact `hello@puentedigital.co.uk`.
4. **Age ratings:** answer the questionnaire (no user-generated content, no purchases, no personal data) → it comes out as "3+" / "Everyone".
5. **Pricing and availability:** Free, all markets.
6. **Store listing:** paste from [listing.md](listing.md). Screenshots: at least one, 1366×768 or larger PNG. Take the settings window on a plain desktop, the overlay with a block highlighted, and a table pasted into Excel.
7. **Submission options → Notes for certification:** paste the note from listing.md (what the shortcuts do, that it's offline, how to test it).
8. Submit. Certification usually takes 1–3 business days. Fix anything they flag and resubmit.

## 5. After it's live

- Put the Store link in the README (the placeholder under *Install*) and on the Puente Digital Labs page.
- New versions: bump the version in `Cargo.toml`, `assets/app.rc` and `assets/app.manifest`, tag, download the new `Nab.msix` from CI, **Update** the submission, upload, submit. Store users get it automatically.

## Later: signing the GitHub .exe too

Once the project has a release history, apply to **SignPath Foundation** (free code signing for open source: https://signpath.org). It needs: MFA on the GitHub account, a "Code signing policy" section in the README naming the maintainer roles, the privacy policy (done), and builds that are reproducible from the public source (done: CI builds from the tag). With that, the `.exe` on GitHub is signed as well and SmartScreen reputation builds from there.
