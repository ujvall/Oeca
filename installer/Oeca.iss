; Oeca Windows installer (Inno Setup 6).
;
; Build the release executable first, then compile this script:
;   cargo build --release --locked
;   "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" installer\Oeca.iss
; Output: target\installer\Oeca-Setup.exe
;
; Per-user install: no administrator rights, installs to
; %LOCALAPPDATA%\Programs\Oeca and registers under HKCU. Nothing runs at
; startup; no services, scheduled tasks or network access.

#define AppName      "Oeca"
#define AppExeName   "Oeca.exe"
#define AppPublisher "Ujval Sai"
#define AppExe       AddBackslash(SourcePath) + "..\target\release\" + AppExeName

#if !FileExists(AppExe)
  #error "target\release\Oeca.exe not found: run `cargo build --release --locked` first"
#endif

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

; Minimal flow: Tasks (shortcuts) -> progress -> Finish
WizardStyle=modern
DisableWelcomePage=yes
DisableDirPage=yes
DisableProgramGroupPage=yes
DisableReadyPage=yes

SetupIconFile=..\assets\branding\Oeca.ico
WizardSmallImageFile=..\assets\branding\Oeca.png
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}

; A running Oeca is offered to be closed (Restart Manager), never restarted
CloseApplications=yes
RestartApplications=no

OutputDir=..\target\installer
OutputBaseFilename=Oeca-Setup
Compression=lzma2/max
SolidCompression=yes

[Messages]
SelectTasksLabel2=Choose the shortcuts to create, then click Install.

[Tasks]
Name: "startmenuicon"; Description: "Create a &Start Menu shortcut"; GroupDescription: "Shortcuts:"
Name: "desktopicon"; Description: "Create a &Desktop shortcut"; GroupDescription: "Shortcuts:"; Flags: unchecked

[Files]
Source: "{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; DestName: "LICENSE.txt"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: startmenuicon
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExeName}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent

[Code]
// With the Ready page skipped, the shortcuts page starts the install: say so
procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpSelectTasks then
    WizardForm.NextButton.Caption := SetupMessage(msgButtonInstall);
end;
