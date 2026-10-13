# Deploying Sintec.PDF on company PCs

Each release has two Windows installers. They install exactly the same way:

| File | What it is |
|---|---|
| `sintec-pdf-<version>-windows-x64.msi` | The Windows Installer package. |
| `sintec-pdf-<version>-windows-x64-setup.exe` | The same signed MSI inside a small launcher, for tools and people that expect an `.exe`. It unpacks the MSI to `%LOCALAPPDATA%\Sintec.PDF\setup` and runs Windows Installer on it, passing msiexec's exit code on. |

Sintec.PDF installs **for all users** (Program Files, HKLM). A silent install therefore needs
administrator rights: deployment tools must run it **as SYSTEM**. Run in a user's context, Windows
asks that user for permission (UAC) whatever switches are given.

## Silent install, upgrade, removal

| Task | MSI | setup.exe |
|---|---|---|
| Install or upgrade | `msiexec /i sintec-pdf-<v>-windows-x64.msi /qn /norestart` | `sintec-pdf-<v>-windows-x64-setup.exe /S` |
| Without the desktop shortcut | add `INSTALLDESKTOPSHORTCUT=0` | add `INSTALLDESKTOPSHORTCUT=0` |
| Remove | `msiexec /x {ProductCode} /qn /norestart` (find the code in the registry under `HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\`, where DisplayName is "Sintec.PDF"), or `msiexec /x sintec-pdf-<v>-windows-x64.msi /qn` with the installed version's MSI | |

`setup.exe` treats `/S`, `/silent`, `/quiet` and `/verysilent` as `/qn /norestart` and passes
any other argument to msiexec unchanged. A new version upgrades the installed one in place (the
old version is removed in the same transaction); installing the same version again does nothing.

Exit codes are msiexec's: **0** done, **3010** done but restart needed, **1602** cancelled,
**1618** another installation is running (retry later), **1603** failed (see the log).

Every install, upgrade and removal writes a verbose log `MSI*.LOG` to the temp folder of the
account that ran it (for SYSTEM: `C:\Windows\Temp`). On a PC where something went wrong, the
diagnostics script collects them:
`irm https://raw.githubusercontent.com/sintec-llc/sintec.pdf/main/packaging/windows/collect-diagnostics.ps1 | iex`

Tested: as SYSTEM through a scheduled task (no desktop, nobody to answer), `setup.exe /S`
upgraded 0.2.2 to 0.2.3, and `msiexec /x … /qn`, `msiexec /i … /qn /norestart` and a repeated
`setup.exe /S` all finished with exit code 0 within seconds and without any prompt.

## ManageEngine Endpoint Central

**Software Deployment → Packages → Add package → Windows:**
- **MSI:** upload the `.msi`. Installation switches: `/qn /norestart`; add
  `INSTALLDESKTOPSHORTCUT=0` if you don't want the desktop shortcut.
- **or EXE:** upload `…-setup.exe`. Installation command: `sintec-pdf-<v>-windows-x64-setup.exe /S`.
- **Deploy through Computer Configuration** (runs as System), not User Configuration (runs as the
  logged-on user, and Windows then asks that user for permission).
- Uninstallation command: `msiexec /x {ProductCode} /qn /norestart`, where `{ProductCode}` is
  different for each release.
- To update: deploy the new release's package the same way; it upgrades the installed version.
- If Sintec.PDF is open during an update, Windows Installer closes it and the update completes; any
  unsaved changes in it are lost, so schedule updates outside working hours.

## Кратко для ИТ (Endpoint Central)

- Пакет **MSI** с ключами `/qn /norestart` или пакет **EXE** с командой `…-setup.exe /S`.
- Развёртывать через **«Конфигурация компьютера»** (от имени System), а не «Конфигурация
  пользователя»: иначе Windows спрашивает разрешение у пользователя.
- Без ярлыка на рабочем столе: добавить `INSTALLDESKTOPSHORTCUT=0`.
- Коды завершения: 0 — готово, 3010 — нужна перезагрузка, 1618 — идёт другая установка (повторить
  позже), 1603 — ошибка (журнал `MSI*.LOG` в `C:\Windows\Temp`).
- Сертификат подписи и правила для антивирусов: [internal-signing.md](internal-signing.md).
