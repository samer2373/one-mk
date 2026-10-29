; Inno Setup script for one-mk (UNTESTED: written on Linux, where Inno Setup
; cannot run). Built by scripts/package-windows.sh:
;   iscc /DAppVersion=0.1.0 /DBinDir=C:\path\to\target\x86_64-pc-windows-msvc\release packaging\windows\one-mk.iss
;
; Installs the three programs to Program Files, adds a Start menu entry,
; opens the Windows firewall for them, puts them on PATH and (optionally)
; starts one-mk at sign-in through `one-mk install`, the same code as
; running it by hand. Uninstalling undoes all of it; the identity and the
; paired computers in %LOCALAPPDATA%\one-mk are the user's and stay.

#ifndef AppVersion
  #error Pass /DAppVersion=x.y.z (scripts/package-windows.sh does)
#endif
#ifndef BinDir
  #error Pass /DBinDir=the folder with one-mk.exe, one-mk-tray.exe and one-mk-app.exe
#endif
#ifndef OutDir
  #define OutDir "..\..\dist"
#endif
#define Brand "..\..\docs\brand\app-icon\windows-linux"

[Setup]
; Never change this GUID: it is how Windows knows an upgrade from a new program.
AppId={{6B0E5B3C-7B0B-4A56-9E2E-1D0C6F6E0A11}
AppName=one-mk
AppVersion={#AppVersion}
AppPublisher=Samer Abu-Sleem
AppPublisherURL=https://github.com/samer2373/one-mk
AppSupportURL=https://github.com/samer2373/one-mk/issues
DefaultDirName={autopf}\one-mk
DefaultGroupName=one-mk
DisableProgramGroupPage=yes
; Program Files and the firewall need an administrator.
PrivilegesRequired=admin
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir={#OutDir}
OutputBaseFilename=one-mk-{#AppVersion}-x64-setup
SetupIconFile={#Brand}\onemk.ico
UninstallDisplayIcon={app}\one-mk-app.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Stops a running one-mk (and the tray) so its files can be replaced, then starts them again.
CloseApplications=force
RestartApplications=yes
ChangesEnvironment=yes

[Tasks]
Name: "startup"; Description: "Start one-mk when I sign in"; GroupDescription: "Startup:"
Name: "addpath"; Description: "Add one-mk to PATH (so `one-mk pair` works in a terminal)"; GroupDescription: "Command line:"

[Files]
Source: "{#BinDir}\one-mk.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\one-mk-tray.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BinDir}\one-mk-app.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\one-mk"; Filename: "{app}\one-mk-app.exe"; Comment: "Share one mouse and keyboard between computers"

[Registry]
; The user's PATH is for them; this is the machine's, so it applies to every account.
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; \
  ValueData: "{olddata};{app}"; Tasks: addpath; Check: NeedsAddPath(ExpandConstant('{app}'))

[Run]
; Firewall (inbound UDP, private and domain networks only, for these programs):
;  - everything one-mk listens on: the link on UDP 24900 and the pairing port, which is chosen at random;
;  - mDNS on UDP 5353, which is how computers find each other.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""one-mk (UDP 24900 and pairing)"" dir=in action=allow protocol=UDP program=""{app}\one-mk.exe"" profile=private,domain"; \
  Flags: runhidden; StatusMsg: "Allowing one-mk through the firewall..."
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""one-mk (mDNS, UDP 5353)"" dir=in action=allow protocol=UDP localport=5353 program=""{app}\one-mk.exe"" profile=private,domain"; \
  Flags: runhidden
; Start at sign-in, as the person who ran Setup (not the elevated account), so it lands in their registry.
Filename: "{app}\one-mk.exe"; Parameters: "install"; Tasks: startup; Flags: runhidden runasoriginaluser; StatusMsg: "Setting up start at sign-in..."
Filename: "{app}\one-mk-app.exe"; Description: "Open one-mk"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Stop it starting again, then stop what runs, then close the firewall.
Filename: "{app}\one-mk.exe"; Parameters: "uninstall"; Flags: runhidden; RunOnceId: "OneMkUninstall"
Filename: "{sys}\taskkill.exe"; Parameters: "/f /im one-mk-app.exe /im one-mk-tray.exe /im one-mk.exe"; Flags: runhidden; RunOnceId: "OneMkStop"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""one-mk (UDP 24900 and pairing)"""; Flags: runhidden; RunOnceId: "OneMkFw1"
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""one-mk (mDNS, UDP 5353)"""; Flags: runhidden; RunOnceId: "OneMkFw2"

[Code]
const
  EnvKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';

function NeedsAddPath(Dir: string): Boolean;
var
  Paths: string;
begin
  if not RegQueryStringValue(HKLM, EnvKey, 'Path', Paths) then
  begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Lowercase(Dir) + ';', ';' + Lowercase(Paths) + ';') = 0;
end;

{ Takes our folder back out of PATH when uninstalling. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Paths, Dir: string;
  P: Integer;
begin
  if CurUninstallStep <> usPostUninstall then exit;
  if not RegQueryStringValue(HKLM, EnvKey, 'Path', Paths) then exit;
  Dir := ExpandConstant('{app}');
  P := Pos(';' + Lowercase(Dir), Lowercase(Paths));
  if P > 0 then
  begin
    Delete(Paths, P, Length(Dir) + 1);
    RegWriteExpandStringValue(HKLM, EnvKey, 'Path', Paths);
  end;
end;
