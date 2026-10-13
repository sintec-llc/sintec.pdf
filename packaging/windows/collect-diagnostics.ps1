<#
.SYNOPSIS
  Collect what's needed to see why a Sintec.PDF installation failed or went missing, into one zip
  on the desktop. Reads only; changes nothing. Run in PowerShell (no administrator rights needed):

    irm https://raw.githubusercontent.com/sintec-llc/sintec.pdf/main/packaging/windows/collect-diagnostics.ps1 | iex

  Collects: Windows Installer and application-error events for Sintec.PDF, the installer's own
  logs (MSI*.LOG in the temp folders, written for every install since 0.2.2) and the updater's,
  Microsoft Defender detections, the program folder, uninstall entries, shortcuts, and
  Sintec.PDF's own log. The zip holds paths and the user name; send it only to the developers.
#>
$ErrorActionPreference = 'SilentlyContinue'
$stamp = Get-Date -Format 'yyyyMMdd-HHmmss'
$out = Join-Path ([Environment]::GetFolderPath('Desktop')) "sintec-diag-$stamp"
New-Item -ItemType Directory -Force $out | Out-Null
function Save($name, $data) { $data | Out-File -FilePath (Join-Path $out $name) -Encoding utf8 -Width 400 }

# System.
Save 'system.txt' @(
  "Collected: $(Get-Date -Format s)"
  "Windows: $([Environment]::OSVersion.VersionString) ($((Get-CimInstance Win32_OperatingSystem).Caption))"
  "64-bit: $([Environment]::Is64BitOperatingSystem)"
  "PowerShell: $($PSVersionTable.PSVersion)"
)

# Windows Installer events (install, upgrade, repair, removal, failures) for Sintec.PDF.
$msi = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; ProviderName = 'MsiInstaller' } -MaxEvents 3000 |
  Where-Object { $_.Message -match 'Sintec' }
Save 'msi-events.txt' ($msi | Select-Object TimeCreated, Id, Message | Format-List | Out-String)

# Crashes and hangs of the app.
$errs = Get-WinEvent -FilterHashtable @{ LogName = 'Application'; Id = 1000, 1001, 1002 } -MaxEvents 2000 |
  Where-Object { $_.Message -match 'sintec-pdf' }
Save 'app-errors.txt' ($errs | Select-Object TimeCreated, Id, Message | Format-List | Out-String)

# Antivirus: Microsoft Defender detections touching Sintec.PDF (other antivirus products keep their own reports).
Save 'defender.txt' @(
  (Get-MpThreatDetection | Where-Object { "$($_.Resources)" -match 'sintec' } | Format-List | Out-String)
  (Get-WinEvent -FilterHashtable @{ LogName = 'Microsoft-Windows-Windows Defender/Operational'; Id = 1116, 1117, 1118, 1119 } -MaxEvents 500 |
    Where-Object { $_.Message -match 'sintec' } | Select-Object TimeCreated, Id, Message | Format-List | Out-String)
  "Antivirus products: " + ((Get-CimInstance -Namespace root/SecurityCenter2 -ClassName AntivirusProduct).displayName -join ', ')
)

# The installer's own logs (MsiLogging, since 0.2.2) and the in-app updater's.
$logs = Join-Path $out 'installer-logs'
New-Item -ItemType Directory -Force $logs | Out-Null
foreach ($dir in @($env:TEMP, "$env:SystemRoot\Temp")) {
  Get-ChildItem $dir -Filter 'MSI*.LOG' | Where-Object { $_.LastWriteTime -gt (Get-Date).AddDays(-30) } | ForEach-Object {
    if (Select-String -LiteralPath $_.FullName -Pattern 'Sintec' -SimpleMatch -Quiet -Encoding Unicode) {
      Copy-Item $_.FullName (Join-Path $logs $_.Name)
    }
  }
}
Get-ChildItem (Join-Path $env:TEMP 'sintec-pdf-update') -Filter '*.log' | Copy-Item -Destination $logs

# What is installed: the folder, the uninstall entries, the shortcuts.
$app = Join-Path $env:ProgramFiles 'Sintec.PDF'
$exe = Join-Path $app 'sintec-pdf.exe'
Save 'install.txt' @(
  "Folder $app exists: $(Test-Path $app)"
  (Get-ChildItem $app -Recurse | Select-Object FullName, Length, LastWriteTime | Format-Table -AutoSize | Out-String)
  "sintec-pdf.exe version: $((Get-Item $exe).VersionInfo.ProductVersion)"
  (Get-ItemProperty 'HKLM:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*', 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\*' |
    Where-Object DisplayName -like '*Sintec*' | Select-Object DisplayName, DisplayVersion, InstallDate, InstallLocation, PSChildName | Format-List | Out-String)
  "Start menu shortcut: $(Test-Path "$env:ProgramData\Microsoft\Windows\Start Menu\Programs\Sintec.PDF.lnk")"
  "Desktop shortcut: $(Test-Path "$env:PUBLIC\Desktop\Sintec.PDF.lnk")"
  "Running now: $((Get-Process sintec-pdf | Select-Object -ExpandProperty Path) -join ', ')"
)

# Sintec.PDF's own log.
Get-ChildItem (Join-Path $env:APPDATA 'Sintec.PDF\data\logs') -Filter '*.log' | Copy-Item -Destination $out

$zip = "$out.zip"
Compress-Archive -Path "$out\*" -DestinationPath $zip -Force
Remove-Item -Recurse -Force $out
Write-Host "Saved: $zip"
Write-Host "Send this file to the Sintec.PDF developers."
