; Inno Setup 6 script — per-user installer for Warp Notes.
; build.ps1 -Installer invokes this with /DMyAppVersion=<cargo version>;
; the fallback below keeps plain `ISCC warp-notes.iss` working too.

#ifndef MyAppVersion
#define MyAppVersion "0.1.0"
#endif

#define MyAppName "Warp Notes"
#define MyAppExeName "warp-notes.exe"

[Setup]
AppId={{7C41E3D8-52A6-4B19-9F0E-2D8B6A5C4E11}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
ArchitecturesInstallIn64BitMode=x64
OutputDir=out
OutputBaseFilename=warp-notes-{#MyAppVersion}-setup-x64
SetupIconFile=warp-notes.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; \
    GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "..\..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; \
    Flags: nowait postinstall skipifsilent
