; Setu for Windows: Inno Setup script. The release workflow builds the programs and runs
;   iscc /DAppVersion=0.1.0 packaging\windows\setu.iss
; Installs for the current user by default (no admin prompt). Uninstalling leaves the
; rules and settings in %APPDATA%\Setu, and every synced file, alone.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define Release "..\..\target\release"

[Setup]
AppId={{6F0B7C55-2E8B-4C1E-9C0A-5E7A3D2B9F14}
AppName=Setu
AppVersion={#AppVersion}
AppVerName=Setu {#AppVersion}
AppPublisher=Setu contributors
AppPublisherURL=https://github.com/sudharamachandranrajeev/setu-sync-manager
DefaultDirName={autopf}\Setu
UsePreviousAppDir=no
DefaultGroupName=Setu
DisableProgramGroupPage=yes
OutputDir=..\..\target\package
OutputBaseFilename=Setu-{#AppVersion}-windows-x64-setup
SetupIconFile=..\..\app\src-tauri\icons\icon.ico
UninstallDisplayIcon={app}\setu-app.exe
UninstallDisplayName=Setu
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
CloseApplications=yes
; Windows 10 1809 or later (WebView2 ships with Windows 11 and current Windows 10)
MinVersion=10.0.17763

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "helper"; Description: "Start Setu's background helper at sign-in (network gate, delete guard, notifications)"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Release}\setu-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Release}\setu.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Release}\setu-watch.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"

[Icons]
; The app ID lets the helper's notifications show as Setu's.
Name: "{autoprograms}\Setu"; Filename: "{app}\setu-app.exe"; Comment: "Choose what syncs, and which way"; AppUserModelID: "dev.setu.app"
Name: "{autodesktop}\Setu"; Filename: "{app}\setu-app.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Setu helper"; ValueData: """{app}\setu-watch.exe"""; Flags: uninsdeletevalue; Tasks: helper

[Run]
Filename: "{app}\setu-watch.exe"; Flags: nowait runhidden; Tasks: helper
Filename: "{app}\setu-app.exe"; Description: "{cm:LaunchProgram,Setu}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "taskkill.exe"; Parameters: "/F /IM setu-watch.exe"; Flags: runhidden; RunOnceId: "StopHelper"

[Code]
// The helper has no window, so Windows cannot ask it to close: stop it before files are replaced.
function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Code: Integer;
begin
  Exec('taskkill.exe', '/F /IM setu-watch.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Result := '';
end;
