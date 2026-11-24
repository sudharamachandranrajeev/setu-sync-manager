; Setu for Windows: Inno Setup script. The release workflow builds the programs and runs
;   iscc /DAppVersion=0.1.0 packaging\windows\setu.iss
; Installs for the current user by default (no admin prompt). Uninstalling leaves the
; rules and settings in %APPDATA%\Setu, the engine's identity and index in
; %LOCALAPPDATA%\Setu\engine, and every synced file, alone.

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
Name: "autostart"; Description: "Start Setu when I sign in (it keeps your folders in sync while it runs)"
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#Release}\setu-app.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#Release}\setu.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"
; The sync engine inside setu-app.exe is Syncthing's code, MPL-2.0.
Source: "..\..\engine\LICENSE-MPL-2.0"; DestDir: "{app}"; DestName: "LICENSE-MPL-2.0.txt"
Source: "..\..\engine\NOTICE"; DestDir: "{app}"; DestName: "NOTICE-engine.txt"

[InstallDelete]
; The helper of 0.1.0-alpha.4 and before; it now runs inside setu-app.exe.
Type: files; Name: "{app}\setu-watch.exe"

[Icons]
; The app ID lets Setu's notifications show as Setu's.
Name: "{autoprograms}\Setu"; Filename: "{app}\setu-app.exe"; Comment: "Choose what syncs, and which way"; AppUserModelID: "dev.setu.app"
Name: "{autodesktop}\Setu"; Filename: "{app}\setu-app.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Setu"; ValueData: """{app}\setu-app.exe"" --background"; Flags: uninsdeletevalue; Tasks: autostart
; The old helper's sign-in entry.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Setu helper"; Flags: deletevalue

[Run]
Filename: "{app}\setu-app.exe"; Description: "{cm:LaunchProgram,Setu}"; Flags: nowait postinstall skipifsilent
; An in-app update runs this setup silently: start Setu again afterwards.
Filename: "{app}\setu-app.exe"; Flags: nowait runasoriginaluser; Check: WizardSilent

[UninstallRun]
Filename: "{app}\setu.exe"; Parameters: "quit"; Flags: runhidden waituntilterminated; RunOnceId: "QuitSetu"

[Code]
// Setu keeps running in the tray, and the old helper has no window, so Windows cannot ask
// either to close. Ask Setu to quit (the engine saves its index), then make sure both are gone.
// `setu quit`, not `setu-app --quit`: an older setu-app ignores the flag and opens its window,
// while an older setu.exe only prints its usage.
procedure StopSetu();
var
  Code: Integer;
begin
  if FileExists(ExpandConstant('{app}\setu.exe')) then
    Exec(ExpandConstant('{app}\setu.exe'), 'quit', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec('taskkill.exe', '/F /IM setu-app.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  Exec('taskkill.exe', '/F /IM setu-watch.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopSetu();
  Result := '';
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if CurUninstallStep = usUninstall then
    Exec('taskkill.exe', '/F /IM setu-app.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;
