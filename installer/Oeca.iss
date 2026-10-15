; Oeca Windows installer. Requires Inno Setup 6.7.0 or later: the dark style
; (6.6.0) plus hidebevels and WizardBackColor (6.7.0); built with 6.7.3.
;
; Build the release executable first, then compile this script:
;   cargo build --release --locked
;   "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\Oeca.iss
; Output: target\installer\Oeca-Setup.exe
;
; Per-user install: no administrator rights, installs to
; %LOCALAPPDATA%\Programs\Oeca and registers under HKCU. The installer adds
; nothing that runs at startup (Oeca's own Start on launch setting is the
; user's choice); no services, scheduled tasks or network access.
;
; Lifecycle:
; - Install / upgrade / reinstall: a running Oeca is asked to quit (its own
;   tray Quit), then the files are replaced in place. Same AppId, folder and
;   shortcut names, so nothing is duplicated; %LOCALAPPDATA%\Oeca (settings,
;   clipboard history) is never touched, and a Start on launch entry keeps
;   pointing at the same, updated executable.
; - Uninstall: Oeca is asked to quit, the installed files, shortcuts and
;   Installed Apps entry are removed, and Oeca's Run entry is removed if it
;   starts this installation (or an executable that no longer exists).
;   User data in %LOCALAPPDATA%\Oeca is kept.
;
; Licensing: LICENSE (as LICENSE.txt) and THIRD-PARTY-NOTICES.txt are installed
; beside Oeca.exe and removed on uninstall. Keep THIRD-PARTY-NOTICES.txt in step
; with the crates compiled in (`cargo tree -e normal`) and the bundled icons.
;
; Appearance: Inno Setup 6.7's built-in dark style (also used by Uninstall),
; Oeca's near-black surfaces, and wizard images rendered from the Oeca icon
; by installer\make-wizard-images.ps1 (several sizes, one per DPI step).

#define AppName      "Oeca"
#define AppExeName   "Oeca.exe"
#define AppPublisher "Ujval Sai"
#define AppExe       AddBackslash(SourcePath) + "..\target\release\" + AppExeName

#if !FileExists(AppExe)
  #error "target\release\Oeca.exe not found: run `cargo build --release --locked` first"
#endif

; Refuse to package a stale build: the executable must be at least as new as
; every input of `cargo build` (sources, manifest, lock file, build script,
; resources). Times compare as yyyymmdd, then hhnnss (ISPP integers are
; 32-bit, too small for the whole stamp).
#define Stamp(str F) GetFileDateTimeString(F, "yyyymmddhhnnss", "", "")
#define Newer(str A, str B) Int(Copy(A, 1, 8)) > Int(Copy(B, 1, 8)) || (Int(Copy(A, 1, 8)) == Int(Copy(B, 1, 8)) && Int(Copy(A, 9, 6)) > Int(Copy(B, 9, 6)))
#define ExeStamp Stamp(AppExe)
#define Repo AddBackslash(SourcePath) + "..\"
#define CheckInput(str F) Newer(Stamp(Repo + F), ExeStamp) ? Error("target\release\Oeca.exe is older than " + F + ": run `cargo build --release --locked` first") : 0
#define FindHandle
#define FindResult
#sub CheckSource
  #expr CheckInput("src\" + FindGetFileName(FindHandle))
#endsub
#for {FindHandle = FindResult = FindFirst(Repo + "src\*.rs", 0); FindResult; FindResult = FindNext(FindHandle)} CheckSource
#if FindHandle
  #expr FindClose(FindHandle)
#endif
#expr CheckInput("Cargo.toml")
#expr CheckInput("Cargo.lock")
#expr CheckInput("build.rs")
#expr CheckInput("assets\windows\oeca.rc")
#expr CheckInput("assets\branding\Oeca.ico")

; Version comes from the executable's own version resource (Cargo.toml)
#define AppVersion   GetStringFileInfo(AppExe, "ProductVersion")
#define AppVersionNumbers GetVersionNumbersString(AppExe)

[Setup]
; Stable identity: keep this AppId for every future release (upgrades and
; uninstall depend on it)
AppId={{3BF695DD-61F2-4C5E-A27F-02FA3DBC01F9}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
VersionInfoVersion={#AppVersionNumbers}
VersionInfoProductName={#AppName}
VersionInfoDescription={#AppName} Setup

; Current user only, never elevated
PrivilegesRequired=lowest
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0

; Flow: Welcome -> Shortcuts (starts the install) -> progress -> Finish
DisableWelcomePage=no
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes

; Dark, flat and quiet: forced dark style, no bevel lines, one charcoal
; surface (the side panel image uses the same colour; keep them in step
; with make-wizard-images.ps1)
WizardStyle=modern dark hidebevels
WizardBackColor=#121214
WizardImageBackColor=#121214
; The header icon's box takes the page colour (no tile behind the icon)
WizardSmallImageBackColor=#121214
WizardImageFile=images\WizardImage-200x384.png,images\WizardImage-250x480.png,images\WizardImage-300x576.png,images\WizardImage-400x768.png
WizardSmallImageFile=images\WizardSmallImage-64.png,images\WizardSmallImage-80.png,images\WizardSmallImage-96.png,images\WizardSmallImage-128.png

SetupIconFile=..\assets\branding\Oeca.ico
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}

; A running Oeca is asked to quit first ([Code]); Restart Manager stays as
; the fallback for anything else holding the files. Never restarted.
CloseApplications=yes
RestartApplications=no

OutputDir=..\target\installer
OutputBaseFilename=Oeca-Setup
Compression=lzma2/max
SolidCompression=yes

[LangOptions]
; Page and welcome titles a step heavier than body text
WelcomeFontName=Segoe UI Semibold
WelcomeFontSize=16

[Messages]
WelcomeLabel1=Welcome to Oeca
WelcomeLabel2=Oeca adds a minimal notch to the top of your screen: the time, what is playing, a timer and your clipboard history, one click away.%n%nIt installs for your Windows account only. No administrator rights are needed.
ClickNext=
WizardSelectTasks=Shortcuts
SelectTasksDesc=Choose where you would like to find Oeca.
SelectTasksLabel2=While it runs, Oeca also has an icon in the notification area.
WizardInstalling=Installing Oeca
InstallingLabel=This only takes a moment.
FinishedHeadingLabel=Oeca is ready
FinishedLabelNoIcons=Oeca is installed. Look for the notch at the top of your screen.
FinishedLabel=Oeca is installed. Look for the notch at the top of your screen, or open Oeca from its shortcut.
ClickFinish=
ConfirmUninstall=Remove %1 from this computer?%n%nYour Oeca settings are kept.
WizardUninstalling=Removing Oeca
UninstallStatusLabel=Please wait while %1 is removed. Your settings are kept.
UninstalledAll=%1 was removed from your computer.

[Tasks]
Name: "startmenuicon"; Description: "Start Menu shortcut"
Name: "desktopicon"; Description: "Desktop shortcut"; Flags: unchecked

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion
; Notices and attributions for the third-party code and icons in Oeca.exe and
; for Inno Setup itself; installed beside LICENSE.txt
Source: "..\THIRD-PARTY-NOTICES.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: startmenuicon
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Open {#AppName} now"; Flags: nowait postinstall skipifsilent

[Code]
const
  OecaWindowClass = 'OecaWindowClass';
  WM_COMMAND = $0111;
  // Oeca's tray command "Quit Oeca" (src/tray.rs): its normal clean shutdown
  OecaQuitCommand = $0102;
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';
  ApprovedKey = 'Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved\Run';
  RunValue = 'Oeca';

// Asks a running Oeca (one per sign-in session) to quit the way its tray
// menu does, and waits up to 5 s for it to finish. Nothing is killed: if it
// does not quit, Setup's usual "files in use" handling still applies.
procedure QuitRunningOeca();
var
  I: Integer;
begin
  if FindWindowByClassName(OecaWindowClass) = 0 then
    Exit;
  PostMessage(FindWindowByClassName(OecaWindowClass), WM_COMMAND, OecaQuitCommand, 0);
  for I := 1 to 50 do
  begin
    Sleep(100);
    if FindWindowByClassName(OecaWindowClass) = 0 then
      Break;
  end;
  // The window closes just before the process ends and releases Oeca.exe
  Sleep(500);
end;

// The executable a Run command starts: the quoted path, or the text up to
// the first space (the same rule as src/startup.rs).
function RunCommandExe(Command: String): String;
begin
  Result := Trim(Command);
  if (Length(Result) > 0) and (Result[1] = '"') then
  begin
    Delete(Result, 1, 1);
    if Pos('"', Result) > 0 then
      Result := Copy(Result, 1, Pos('"', Result) - 1);
  end
  else if Pos(' ', Result) > 0 then
    Result := Copy(Result, 1, Pos(' ', Result) - 1);
end;

// Removes Oeca's Start on launch entry (and Task Manager's mark for it) if
// it starts this installation or an executable that no longer exists. An
// entry for another, existing Oeca (e.g. a development build) is kept.
// Missing entries are fine (already removed, or never enabled).
procedure RemoveOwnStartupEntry();
var
  Command, Exe: String;
begin
  if not RegQueryStringValue(HKCU, RunKey, RunValue, Command) then
    Exit;
  Exe := RunCommandExe(Command);
  if (CompareText(Exe, ExpandConstant('{app}\{#AppExeName}')) = 0) or not FileExists(Exe) then
  begin
    RegDeleteValue(HKCU, RunKey, RunValue);
    RegDeleteValue(HKCU, ApprovedKey, RunValue);
  end;
end;

// Install, upgrade or reinstall: Oeca must not be running while its
// executable is replaced
function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  QuitRunningOeca();
  Result := '';
end;

// Uninstall, after the user confirmed: quit Oeca before its files go; once
// they are gone, drop its startup entry. User data is left in place.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usUninstall then
    QuitRunningOeca();
  if CurUninstallStep = usPostUninstall then
    RemoveOwnStartupEntry();
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  // With the Ready page skipped, the shortcuts page starts the install: say so
  if CurPageID = wpSelectTasks then
    WizardForm.NextButton.Caption := SetupMessage(msgButtonInstall);
  // Progress shows the step ("Creating shortcuts...") without the full path
  // of every file under it
  if CurPageID = wpInstalling then
    WizardForm.FilenameLabel.Visible := False;
end;
