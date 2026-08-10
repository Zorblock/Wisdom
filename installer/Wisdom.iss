#define MyAppName "Wisdom"
#define MyAppVersion "0.1.0"
#define MyAppPublisher "zorblock"
#define MyAppExeName "wisdom.exe"

[Setup]
AppId={{B9B4C617-A7CE-45FC-B53D-424BF27075EA}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={userappdata}\zorblock\Wisdom
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=output
OutputBaseFilename=Wisdom-Setup
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
SetupIconFile=..\assets\wisdom.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
ArchitecturesInstallIn64BitMode=x64compatible

[Files]
Source: "..\target\release\wisdom.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Wisdom"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\Wisdom"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch Wisdom"; Flags: nowait postinstall skipifsilent
