; Inno Setup script for the Ratchet installer.
; Build after packaging/windows/bundle.sh has filled target\dist:
;   iscc /DAppVersion=1.0.0 packaging\windows\ratchet.iss
; The installer lands in target\installer.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
; Keep AppId fixed: Windows uses it to recognize updates and the uninstaller.
AppId={{2334DE1F-E7BC-477F-950E-25E6EE6A9B19}
AppName=Ratchet
AppVersion={#AppVersion}
AppVerName=Ratchet {#AppVersion}
AppPublisher=R3ktline
AppPublisherURL=https://github.com/R3ktline/Ratchet
AppSupportURL=https://github.com/R3ktline/Ratchet/issues
AppUpdatesURL=https://github.com/R3ktline/Ratchet/releases
DefaultDirName={autopf}\Ratchet
DefaultGroupName=Ratchet
DisableProgramGroupPage=yes
; Installs for the current user without admin rights; the first page offers
; installing for all users instead.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
LicenseFile=..\..\LICENSE
SetupIconFile=..\..\data\windows\ratchet.ico
UninstallDisplayIcon={app}\bin\ratchet.exe
UninstallDisplayName=Ratchet
OutputDir=..\..\target\installer
OutputBaseFilename=Ratchet-{#AppVersion}-x64-setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[InstallDelete]
; Drop files of the previous version, so stale DLLs never mix with new ones.
Type: filesandordirs; Name: "{app}\bin"
Type: filesandordirs; Name: "{app}\lib"
Type: filesandordirs; Name: "{app}\share"
Type: filesandordirs; Name: "{app}\etc"

[Files]
Source: "..\..\target\dist\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\Ratchet"; Filename: "{app}\bin\ratchet.exe"; WorkingDir: "{app}\bin"
Name: "{autodesktop}\Ratchet"; Filename: "{app}\bin\ratchet.exe"; WorkingDir: "{app}\bin"; Tasks: desktopicon

[Run]
Filename: "{app}\bin\ratchet.exe"; Description: "{cm:LaunchProgram,Ratchet}"; Flags: nowait postinstall skipifsilent
