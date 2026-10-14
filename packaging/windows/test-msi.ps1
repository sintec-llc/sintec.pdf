<#
.SYNOPSIS
  Check the compiled MSI's shortcuts, desktop-shortcut checkbox and full-UI outcome wiring (#143), without installing it.
.EXAMPLE
  pwsh packaging/windows/test-msi.ps1 dist/release/sintec-pdf-0.1.3-windows-x64.msi
#>
param([Parameter(Mandatory)] [string] $Path)
$ErrorActionPreference = 'Stop'
$Installer = New-Object -ComObject WindowsInstaller.Installer
$Database = $Installer.OpenDatabase((Resolve-Path -LiteralPath $Path).Path, 0)

function Read-Row([string] $Sql, [int] $Columns) {
  $view = $Database.OpenView($Sql)
  try {
    [void] $view.Execute()
    $record = $view.Fetch()
    if (-not $record) { throw "MSI row missing: $Sql" }
    $values = @(for ($i = 1; $i -le $Columns; $i++) { $record.StringData($i) })
    return ,$values
  } finally { [void] $view.Close() }
}

function Assert-Equal($Actual, $Expected, [string] $What) {
  if ($Actual -cne $Expected) { throw "$What`: expected '$Expected', got '$Actual'" }
}

# Plain (non-advertised) shortcuts to sintec-pdf.exe, each in its own component; the desktop one is
# gated by INSTALLDESKTOPSHORTCUT, which defaults to 1 and is secure so the UI choice reaches the
# elevated install.
foreach ($entry in @(@('StartMenuShortcut', 'ProgramMenuFolder', 'PdfcraftStartMenuShortcut', ''),
                     @('DesktopShortcut', 'DesktopFolder', 'PdfcraftDesktopShortcut', 'INSTALLDESKTOPSHORTCUT = 1'))) {
  $row = Read-Row ('SELECT `Directory_`, `Target`, `Icon_`, `WkDir`, `Component_` FROM `Shortcut` WHERE `Shortcut` = ''' + $entry[0] + '''') 5
  Assert-Equal $row[0] $entry[1] "$($entry[0]) directory"
  Assert-Equal $row[1] '[#PdfcraftExe]' "$($entry[0]) target"
  Assert-Equal $row[2] 'PdfcraftIcon.ico' "$($entry[0]) icon"
  Assert-Equal $row[3] 'INSTALLFOLDER' "$($entry[0]) working directory"
  Assert-Equal $row[4] $entry[2] "$($entry[0]) component"
  $component = Read-Row ('SELECT `Directory_`, `Condition` FROM `Component` WHERE `Component` = ''' + $entry[2] + '''') 2
  Assert-Equal $component[0] $entry[1] "$($entry[2]) directory"
  Assert-Equal $component[1] $entry[3] "$($entry[2]) condition"
}
$default = Read-Row 'SELECT `Value` FROM `Property` WHERE `Property` = ''INSTALLDESKTOPSHORTCUT''' 1
Assert-Equal $default[0] '1' 'Desktop shortcut default'
$secure = Read-Row 'SELECT `Value` FROM `Property` WHERE `Property` = ''SecureCustomProperties''' 1
if (($secure[0] -split ';') -notcontains 'INSTALLDESKTOPSHORTCUT') { throw "INSTALLDESKTOPSHORTCUT is not secure: '$($secure[0])'" }
$checkbox = Read-Row 'SELECT `Type`, `Property`, `Text` FROM `Control` WHERE `Dialog_` = ''InstallWelcome'' AND `Control` = ''DesktopShortcut''' 3
Assert-Equal $checkbox[0] 'CheckBox' 'Welcome desktop-shortcut control'
Assert-Equal $checkbox[1] 'INSTALLDESKTOPSHORTCUT' 'Welcome checkbox property'
if ($checkbox[2] -notmatch 'desktop shortcut') { throw "Welcome checkbox label: '$($checkbox[2])'" }
$checked = Read-Row 'SELECT `Value` FROM `CheckBox` WHERE `Property` = ''INSTALLDESKTOPSHORTCUT''' 1
Assert-Equal $checked[0] '1' 'Welcome checkbox value'
$app = Read-Row 'SELECT `KeyPath` FROM `Component` WHERE `Component` = ''PdfcraftApp''' 1
Assert-Equal $app[0] 'PdfcraftExe' 'Shortcut executable key path'
$scope = Read-Row 'SELECT `Value` FROM `Property` WHERE `Property` = ''ALLUSERS''' 1
Assert-Equal $scope[0] '1' 'Per-machine shortcut scope'

# Explorer «Преобразовать в PDF» on PDFs, images, text files and Office documents, for any number of files; the app
# component owns every registry row so uninstall removes it. No default association is changed.
foreach ($ext in @('pdf', 'png', 'jpg', 'jpeg', 'tif', 'tiff', 'gif', 'bmp', 'jp2', 'j2k', 'jpx', 'txt', 'text', 'md', 'csv', 'log', 'ini', 'cfg', 'json', 'xml', 'yml', 'yaml', 'doc', 'docx', 'docm', 'dot', 'dotx', 'rtf', 'odt', 'xls', 'xlsx', 'xlsm', 'xlsb', 'ods', 'ppt', 'pptx', 'pptm', 'pps', 'ppsx', 'odp')) {
  $key = 'Software\Classes\SystemFileAssociations\.' + $ext + '\shell\SintecPDF.ConvertToPdf'
  $menu = Read-Row ('SELECT `Value`, `Component_`, `Root` FROM `Registry` WHERE `Key` = ''' + $key + ''' AND `Name` IS NULL') 3
  Assert-Equal $menu[0] 'Преобразовать в PDF' "$ext context menu label"
  Assert-Equal $menu[1] 'PdfcraftApp' "$ext context menu component"
  Assert-Equal $menu[2] '2' "$ext context menu HKLM root"
  $command = Read-Row ('SELECT `Value` FROM `Registry` WHERE `Key` = ''' + $key + '\command''') 1
  Assert-Equal $command[0] '"[#PdfcraftExe]" --convert-to-pdf "%1"' "$ext context menu command"
  $selection = Read-Row ('SELECT `Value` FROM `Registry` WHERE `Key` = ''' + $key + ''' AND `Name` = ''MultiSelectModel''') 1
  Assert-Equal $selection[0] 'Player' "$ext context menu selection"
}

# PDF files show the PDF file icon (exe resource 2), not the app logo, and so does "Open with".
foreach ($key in @('Software\Classes\SintecPDF.Document\DefaultIcon', 'Software\Classes\Applications\sintec-pdf.exe\DefaultIcon')) {
  $icon = Read-Row ('SELECT `Value`, `Component_` FROM `Registry` WHERE `Key` = ''' + $key + ''' AND `Name` IS NULL') 2
  Assert-Equal $icon[0] '"[#PdfcraftExe]",-2' $key
  Assert-Equal $icon[1] 'PdfcraftApp' "$key component"
}
# Licences installed beside the exe.
$names = @('LICENSE-MIT', 'LICENSE-APACHE', 'NOTICE', 'LICENSE-document.txt', 'OFL-Inter.txt', 'OFL-JetBrainsMono.txt', 'OFL-DancingScript.txt', 'OFL-MarckScript.txt')
for ($i = 0; $i -lt $names.Count; $i++) {
  $file = Read-Row ('SELECT `FileName`, `Component_` FROM `File` WHERE `File` = ''LicenceFile' + $i + '''') 2
  if (($file[0] -split '\|')[-1] -cne $names[$i]) { throw "LicenceFile$i`: expected $($names[$i]), got '$($file[0])'" }
  $dir = Read-Row ('SELECT `Directory_` FROM `Component` WHERE `Component` = ''' + $file[1] + '''') 1
  Assert-Equal $dir[0] 'INSTALLFOLDER' "$($names[$i]) directory"
}
# Every install, upgrade and removal writes a verbose log to %TEMP%, however it was started.
$logging = Read-Row 'SELECT `Value` FROM `Property` WHERE `Property` = ''MsiLogging''' 1
Assert-Equal $logging[0] 'voicewarmup' 'Automatic installer log'
# Never per-user: HKLM registrations must not point into one user's profile.
$launch = Read-Row 'SELECT `Condition` FROM `LaunchCondition`' 1
Assert-Equal $launch[0] 'ALLUSERS = 1 OR (Installed AND REMOVE = "ALL")' 'Per-machine launch condition'
# PDF context menu (#682): "Edit with PdfCraft" opens the file in PdfCraft. Its own component, gated
# by INSTALLPDFCONTEXTMENU (default 1, secure, welcome checkbox), with its menu value as key path so
# uninstall removes it. Several selected PDFs each start pdfcraft.exe "%1" (Document), which the
# single-instance hand-off opens as tabs. The .pdf default is never set, so the user's default app
# stays.
$pdfKey = 'Software\Classes\SystemFileAssociations\.pdf\shell\PdfCraft.Edit'
$pdfMenu = Read-Row ('SELECT `Registry`, `Value`, `Component_`, `Root` FROM `Registry` WHERE `Key` = ''' + $pdfKey + ''' AND `Name` IS NULL') 4
Assert-Equal $pdfMenu[1] 'Edit with PdfCraft' 'PDF context menu label'
Assert-Equal $pdfMenu[2] 'PdfcraftPdfContextMenu' 'PDF context menu component'
Assert-Equal $pdfMenu[3] '2' 'PDF context menu HKLM root'
foreach ($value in @(@('Icon', '"[#PdfcraftExe]",0'), @('MultiSelectModel', 'Document'))) {
  $row = Read-Row ('SELECT `Value`, `Component_` FROM `Registry` WHERE `Key` = ''' + $pdfKey + ''' AND `Name` = ''' + $value[0] + '''') 2
  Assert-Equal $row[0] $value[1] "PDF context menu $($value[0])"
  Assert-Equal $row[1] 'PdfcraftPdfContextMenu' "PDF context menu $($value[0]) component"
}
$pdfCommand = Read-Row ('SELECT `Value`, `Component_` FROM `Registry` WHERE `Key` = ''' + $pdfKey + '\command''') 2
Assert-Equal $pdfCommand[0] '"[#PdfcraftExe]" "%1"' 'PDF context menu command'
Assert-Equal $pdfCommand[1] 'PdfcraftPdfContextMenu' 'PDF context menu command component'
$pdfComponent = Read-Row 'SELECT `Condition`, `KeyPath` FROM `Component` WHERE `Component` = ''PdfcraftPdfContextMenu''' 2
Assert-Equal $pdfComponent[0] 'INSTALLPDFCONTEXTMENU = 1' 'PDF context menu condition'
Assert-Equal $pdfComponent[1] $pdfMenu[0] 'PDF context menu key path'
$pdfDefault = Read-Row 'SELECT `Value` FROM `Property` WHERE `Property` = ''INSTALLPDFCONTEXTMENU''' 1
Assert-Equal $pdfDefault[0] '1' 'PDF context menu default'
if (($secure[0] -split ';') -notcontains 'INSTALLPDFCONTEXTMENU') { throw "INSTALLPDFCONTEXTMENU is not secure: '$($secure[0])'" }
$pdfCheckbox = Read-Row 'SELECT `Type`, `Property`, `Text` FROM `Control` WHERE `Dialog_` = ''InstallWelcome'' AND `Control` = ''PdfContextMenu''' 3
Assert-Equal $pdfCheckbox[0] 'CheckBox' 'Welcome PDF context-menu control'
Assert-Equal $pdfCheckbox[1] 'INSTALLPDFCONTEXTMENU' 'Welcome PDF context-menu property'
if ($pdfCheckbox[2] -notmatch 'Edit with PdfCraft') { throw "Welcome PDF context-menu label: '$($pdfCheckbox[2])'" }
$pdfChecked = Read-Row 'SELECT `Value` FROM `CheckBox` WHERE `Property` = ''INSTALLPDFCONTEXTMENU''' 1
Assert-Equal $pdfChecked[0] '1' 'Welcome PDF context-menu value'
Assert-NoRow 'SELECT `Registry` FROM `Registry` WHERE `Key` = ''Software\Classes\.pdf'' AND `Name` IS NULL' '.pdf default app'

# Negative sequences are Windows Installer's success/user-exit/failure paths. Only full UI
# shows these dialogs: an unattended /qn or /qb install must never wait for a Finish click.
foreach ($exit in @(@('InstallComplete', '-1'), @('InstallCancelled', '-2'), @('InstallFailed', '-3'))) {
  $row = Read-Row ('SELECT `Condition`, `Sequence` FROM `InstallUISequence` WHERE `Action` = ''' + $exit[0] + '''') 2
  Assert-Equal $row[0] 'UILevel = 5' "$($exit[0]) UI level"
  Assert-Equal $row[1] $exit[1] "$($exit[0]) exit path"
  $finish = Read-Row ('SELECT `Type`, `Text` FROM `Control` WHERE `Dialog_` = ''' + $exit[0] + ''' AND `Control` = ''Finish''') 2
  Assert-Equal $finish[0] 'PushButton' "$($exit[0]) Finish control"
  Assert-Equal $finish[1] '&Finish' "$($exit[0]) Finish label"
  $endDialog = Read-Row ('SELECT `Argument` FROM `ControlEvent` WHERE `Dialog_` = ''' + $exit[0] + ''' AND `Control_` = ''Finish'' AND `Event` = ''EndDialog''') 1
  Assert-Equal $endDialog[0] 'Return' "$($exit[0]) Finish event"
}
$title = Read-Row 'SELECT `Text` FROM `Control` WHERE `Dialog_` = ''InstallComplete'' AND `Control` = ''Title''' 1
if ($title[0] -notmatch 'completed successfully') { throw 'Success dialog does not confirm completion' }
$rm = Read-Row 'SELECT `Dialog` FROM `Dialog` WHERE `Dialog` = ''MsiRMFilesInUse''' 1
Assert-Equal $rm[0] 'MsiRMFilesInUse' 'Files-in-use dialog'
[void] [Runtime.InteropServices.Marshal]::FinalReleaseComObject($Database)
[void] [Runtime.InteropServices.Marshal]::FinalReleaseComObject($Installer)
Write-Output 'ok MSI: Start Menu shortcut, «Преобразовать в PDF» context menu, optional desktop shortcut (default on, checkbox), icon/key path, full-UI success/cancel/error and Finish controls, files-in-use dialog, PDF file icon, licences, per-machine only'
