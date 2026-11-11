; Inno Setup 6 script — per-user installer for Warp Notes.
; build.ps1 passes /DMyAppVersion=<suite version from version.toml>; a manual
; `ISCC warp-notes.iss` must pass it too, so there is no stale fallback here.

#ifndef MyAppVersion
  #error MyAppVersion is required: pass /DMyAppVersion=<version from version.toml>
#endif

#define MyAppName "Warp Notes"
#define MyAppExeName "warp-notes.exe"

; build.ps1 passes the staging dir and the dist output dir; the fallbacks keep
; a manual `iscc warp-notes.iss` working against packaging\windows\out.
#ifndef MySourceDir
#define MySourceDir "out"
#endif
#ifndef MyOutputDir
#define MyOutputDir "out"
#endif

[Setup]
AppId={{7C41E3D8-52A6-4B19-9F0E-2D8B6A5C4E11}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
PrivilegesRequired=lowest
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#MyOutputDir}
OutputBaseFilename=warp-notes-{#MyAppVersion}-setup-x64
SetupIconFile=warp-notes.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; \
    GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#MySourceDir}\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#MyAppName}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{autodesktop}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{cm:LaunchProgram,{#MyAppName}}"; \
    Flags: nowait postinstall skipifsilent
