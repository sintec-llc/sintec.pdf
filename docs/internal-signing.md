# Internal code signing (Sintec LLC)

Sintec.PDF's Windows files (`sintec-pdf.exe`, `sintec-pdf-cli.exe` and the MSI) are signed with
**Sintec LLC's internal code-signing certificate**. It is self-signed: company PCs trust it once IT
has distributed it (below), and antivirus products can then treat Sintec.PDF as trusted software.
Outside the company the files count as unsigned.

| | |
|---|---|
| Subject | `CN=Sintec LLC, O=Sintec LLC, C=RU` |
| Thumbprint (SHA-1) | `21F3F54B13DCBF3F661883F35BDCE29F27E55C7B` |
| Valid | 2026-10-13 to 2031-10-13 |
| Key | RSA 3072, SHA-256, code signing only |
| Public certificate | [`packaging/windows/sintec-codesign.cer`](../packaging/windows/sintec-codesign.cer) |

Every signature is timestamped (DigiCert), so files signed while the certificate is valid stay valid
after it expires.

## Кратко для ИТ-отдела

Сертификат нужно один раз добавить на компьютеры компании в два хранилища: **«Доверенные корневые
центры сертификации»** и **«Доверенные издатели»**.

**Через групповую политику (домен):**
1. «Управление групповой политикой» → политика, применяемая к компьютерам компании → «Изменить».
2. «Конфигурация компьютера» → «Политики» → «Конфигурация Windows» → «Параметры безопасности» →
   «Политики открытого ключа».
3. «Доверенные корневые центры сертификации» → правый щелчок → «Импорт» → `sintec-codesign.cer`.
4. «Доверенные издатели» → правый щелчок → «Импорт» → тот же файл.
5. На компьютерах: `gpupdate /force` (или дождаться обновления политики).

**На отдельном компьютере** (PowerShell от имени администратора, в папке с файлом):
```powershell
Import-Certificate -FilePath .\sintec-codesign.cer -CertStoreLocation Cert:\LocalMachine\Root
Import-Certificate -FilePath .\sintec-codesign.cer -CertStoreLocation Cert:\LocalMachine\TrustedPublisher
```

**Антивирус.** Добавьте Sintec.PDF в доверенные по цифровой подписи (издатель «Sintec LLC», отпечаток
`21F3F54B13DCBF3F661883F35BDCE29F27E55C7B`); если правило по сертификату недоступно, добавьте
исключение для папки `C:\Program Files\Sintec.PDF\` и для файла установщика:
- **Kaspersky Security Center / Kaspersky Endpoint Security:** в политике — доверенные программы /
  исключения (правило по цифровому сертификату или по пути).
- **ESET PROTECT:** в политике — исключения (по пути) и, при необходимости, правило HIPS для
  установщика.

Без этого антивирус может прервать установку на середине (так было 13.10.2026 на ПК с ESET и
Kaspersky одновременно: старая версия удалилась, новая не доставилась). Держать на одном ПК два
антивируса не рекомендуется.

**Проверка:** правый щелчок по `sintec-pdf.exe` → «Свойства» → «Цифровые подписи» → «Sintec LLC»,
«Эта цифровая подпись действительна».

## How the build signs

`packaging/windows/sign.ps1` signs with the `.pfx` in the `WINDOWS_CERTIFICATE` secret (base64) and
`WINDOWS_CERTIFICATE_PASSWORD`, both in the repository's `release` environment. The CI runner doesn't
trust the internal root, so instead of `signtool verify /pa` the script checks, for a self-signed
certificate, that each file carries an intact, timestamped signature by that very certificate.
Builds without the secrets (local or CI test builds) stay unsigned, as before.

## Keeping the key

The private key (`sintec-codesign.pfx`) and its password were created on the owner's PC, in
`Documents\Sintec.PDF code signing`, and stored as the two `release` secrets.
- Keep a backup of the `.pfx` somewhere safe and offline. Move the password from `PFX-PASSWORD.txt`
  into a password manager and delete the text file.
- **If the key leaks:** create a new certificate the same way, replace the two secrets, have IT
  import the new `.cer` and **remove the old one** from both stores (thumbprint above), then
  re-release.
- **Before 2031-10-13:** create the next certificate and roll it out the same way.

Creating a certificate (PowerShell, no admin rights needed):
```powershell
$cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject 'CN=Sintec LLC, O=Sintec LLC, C=RU' `
  -KeyAlgorithm RSA -KeyLength 3072 -HashAlgorithm SHA256 -KeyExportPolicy Exportable `
  -KeyUsage DigitalSignature -NotAfter (Get-Date).AddYears(5) -CertStoreLocation Cert:\CurrentUser\My
Export-PfxCertificate -Cert $cert -FilePath sintec-codesign.pfx -Password (Read-Host -AsSecureString 'PFX password')
Export-Certificate -Cert $cert -FilePath sintec-codesign.cer
Remove-Item "Cert:\CurrentUser\My\$($cert.Thumbprint)" -DeleteKey
```
If the company runs its own certificate authority (AD CS), a code-signing certificate issued by it
works too: export it with its key as `.pfx` and put it in the same two secrets.
