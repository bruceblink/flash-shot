#define MyAppName "Flash Shot"
#define MyAppPublisher "bruceblink"
#define MyAppExeName "flash-shot.exe"

#ifndef MyAppVersion
  #error MyAppVersion must be supplied by scripts/package-installer.ps1
#endif

#ifndef MySourceDir
  #error MySourceDir must be supplied by scripts/package-installer.ps1
#endif

[Setup]
AppId={{BF3C499B-7D1B-4E5D-9E9B-7BF1A1E9297D}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
UninstallDisplayName={#MyAppName}
OutputDir=.
OutputBaseFilename=FlashShot-{#MyAppVersion}-windows-setup
SetupIconFile=..\resources\icons\icon.ico
Compression=lzma2/ultra64
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=admin
DisableProgramGroupPage=yes
LicenseFile={#MySourceDir}\LICENSE.txt

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "chinesesimp"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#MySourceDir}\flash-shot.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MySourceDir}\LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MySourceDir}\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#MySourceDir}\PORTABLE.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "Launch {#MyAppName}"; Flags: nowait postinstall skipifsilent
