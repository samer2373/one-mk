; Windows installer, built by CI with Inno Setup:
;   iscc /DAppVersion=0.1.1 /DReleaseDir=target\x86_64-pc-windows-msvc\release installer\one-mk.iss
; Installs for the current user only (no administrator rights needed).
[Setup]
AppId={{6B1F3C52-0D4A-4E8B-9C7A-1E2D5A4F7B90}
AppName=one·mk
AppVersion={#AppVersion}
AppPublisher=Samer Abu-Sleem
AppPublisherURL=https://github.com/samer2373/one-mk
DefaultDirName={localappdata}\Programs\one-mk
DefaultGroupName=one·mk
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..
OutputBaseFilename=one-mk-x86_64-pc-windows-msvc-setup
SetupIconFile=..\docs\brand\app-icon\windows-linux\onemk.ico
UninstallDisplayIcon={app}\one-mk-app.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "addpath"; Description: "Add one-mk to PATH, to use it from a terminal"

[Files]
Source: "{#ReleaseDir}\one-mk.exe"; DestDir: "{app}"
Source: "{#ReleaseDir}\one-mk-tray.exe"; DestDir: "{app}"
Source: "{#ReleaseDir}\one-mk-app.exe"; DestDir: "{app}"

[Icons]
Name: "{group}\one·mk"; Filename: "{app}\one-mk-app.exe"
Name: "{autodesktop}\one·mk"; Filename: "{app}\one-mk-app.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addpath; Check: NeedsPath(ExpandConstant('{app}'))

[Run]
Filename: "{app}\one-mk-app.exe"; Description: "Open one·mk"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Takes out the Explorer "Send to" entries `one-mk integrate` wrote (only
; files carrying its marker; a user's own are left alone), which would
; otherwise point at the deleted one-mk.exe. [UninstallRun] runs before any
; file is removed, as the user who installed (this installer never asks for
; administrator rights), hidden. Its exit code is not checked, so a failure
; here never stops the uninstall.
Filename: "{app}\one-mk.exe"; Parameters: "integrate --remove"; Flags: runhidden waituntilterminated skipifdoesntexist; RunOnceId: "RemoveSendTo"

[Code]
function NeedsPath(Dir: string): Boolean;
var
  Paths: string;
begin
  if not RegQueryStringValue(HKCU, 'Environment', 'Path', Paths) then
    Paths := '';
  Result := Pos(';' + Lowercase(Dir) + ';', ';' + Lowercase(Paths) + ';') = 0;
end;
