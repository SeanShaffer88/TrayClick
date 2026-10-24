; TrayClick installer (Inno Setup 6).
; Built by the Release workflow: ISCC /DAppVersion=<x.y.z> /DTagName=<vX.Y.Z> installer\TrayClick.iss
; Produces dist\TrayClick-Setup-<TagName>-x64.exe — a per-user install
; (no admin needed). The portable download is the same trayclick.exe,
; published next to this installer on every release.

#ifndef AppVersion
  #define AppVersion "0.2.1"
#endif
#ifndef TagName
  #define TagName "v0.2.1"
#endif

#define AppName "TrayClick"

[Setup]
AppId={{B7C4A2E1-5D38-4F6A-9C21-8E0D3A5F7B42}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=Sean Shaffer
AppPublisherURL=https://github.com/SeanShaffer88/TrayClick
AppSupportURL=https://github.com/SeanShaffer88/TrayClick
DefaultDirName={autopf}\TrayClick
DefaultGroupName=TrayClick
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename=TrayClick-Setup-{#TagName}-x64
SetupIconFile=..\assets\icon.ico
Compression=lzma2
SolidCompression=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
UninstallDisplayIcon={app}\trayclick.exe
UninstallDisplayName=TrayClick
WizardStyle=modern

[Files]
Source: "..\target\release\trayclick.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\TrayClick"; Filename: "{app}\trayclick.exe"
Name: "{group}\Uninstall TrayClick"; Filename: "{uninstallexe}"

[Run]
Filename: "{app}\trayclick.exe"; Description: "Launch TrayClick now"; Flags: nowait postinstall skipifsilent
