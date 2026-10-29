; Inno Setup script for one-mk (UNTESTED: written on Linux, where Inno Setup
; cannot run). Built by scripts/package-windows.sh:
;   iscc /DAppVersion=0.1.0 /DBinDir=C:\path\to\target\x86_64-pc-windows-msvc\release packaging\windows\one-mk.iss
;
; Installs the three programs to Program Files, adds a Start menu entry,
; opens the Windows firewall for them, puts them on PATH and (optionally)
; starts one-mk at sign-in through `one-mk install`, the same code as
; running it by hand. Uninstalling undoes all of it; the identity and the
; paired computers in %LOCALAPPDATA%\one-mk are the user's and stay.
; The PATH and startup-entry logic is in path-entries.pas (included below),
; which scripts/check-windows-path.sh runs, step for step, on a list of cases.

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
  { What this installation did to the machine, so that uninstalling takes back
    only that: PathEntry is the folder it added to PATH (not written if the
    folder was on PATH already). Removed again on uninstall. }
  StateKey = 'SOFTWARE\one-mk';
  ProfileListKey = 'SOFTWARE\Microsoft\Windows NT\CurrentVersion\ProfileList';
  RunSubKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

#include "path-entries.pas"

{ The machine's PATH (the user's PATH is theirs): our folder is added as a
  whole entry, once, and only recorded as ours if it was not there before. }
procedure CurStepChanged(CurStep: TSetupStep);
var
  Paths, Dir: string;
begin
  if (CurStep <> ssPostInstall) or not WizardIsTaskSelected('addpath') then exit;
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKLM, EnvKey, 'Path', Paths) then Paths := '';
  if PathHasEntry(Paths, Dir) then exit;
  RegWriteExpandStringValue(HKLM, EnvKey, 'Path', PathWithEntry(Paths, Dir));
  RegWriteStringValue(HKLM, StateKey, 'PathEntry', Dir);
end;

{ Takes our folder back out of PATH, if this installation put it there:
  whole entries only, so the neighbours of ours are never touched. }
procedure RemovePathEntry;
var
  Paths, Added, Dir: string;
begin
  Dir := ExpandConstant('{app}');
  if not RegQueryStringValue(HKLM, StateKey, 'PathEntry', Added) then exit;
  if Lowercase(Added) <> Lowercase(Dir) then exit;
  if RegQueryStringValue(HKLM, EnvKey, 'Path', Paths) and PathHasEntry(Paths, Dir) then
    RegWriteExpandStringValue(HKLM, EnvKey, 'Path', PathWithoutEntry(Paths, Dir));
end;

{ `one-mk uninstall` (in [UninstallRun]) removes the start-at-sign-in entry of
  the account that is running the uninstaller. Setup registered it for whoever
  started Setup (runasoriginaluser), which is another account if a standard
  user installed with an administrator's credentials. So every user's startup
  entry that starts a program from this folder is removed from the loaded user
  registry (HKEY_USERS\<SID>), and for a profile whose registry is not loaded
  (the user is signed out) the uninstall log says what to do by hand. }
procedure RemoveStartupEntries;
var
  Sids: TArrayOfString;
  I: Integer;
  Dir, RunKey, Data: string;
begin
  Dir := ExpandConstant('{app}');
  if not RegGetSubkeyNames(HKLM, ProfileListKey, Sids) then exit;
  for I := 0 to GetArrayLength(Sids) - 1 do
  begin
    { Real accounts only: local, domain (S-1-5-21-...) and Microsoft/Azure AD (S-1-12-1-...). }
    if (Pos('S-1-5-21-', Sids[I]) <> 1) and (Pos('S-1-12-1-', Sids[I]) <> 1) then continue;
    RunKey := Sids[I] + '\' + RunSubKey;
    if not RegKeyExists(HKU, Sids[I]) then
      Log('The registry of ' + Sids[I] + ' is not loaded (that user is signed out), so a start-at-sign-in entry of theirs was not looked for. If they used it, remove the value one-mk from HKEY_USERS\' + Sids[I] + '\' + RunSubKey + ' (or run `one-mk uninstall` as that user).')
    else if RegQueryStringValue(HKU, RunKey, 'one-mk', Data) and RunValueIsOurs(Data, Dir) then
    begin
      if RegDeleteValue(HKU, RunKey, 'one-mk') then
        Log('Removed the start-at-sign-in entry of ' + Sids[I])
      else
        Log('Could not remove the start-at-sign-in entry of ' + Sids[I] + ' (HKEY_USERS\' + RunKey + '\one-mk); remove it by hand.');
    end;
  end;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep <> usPostUninstall then exit;
  RemoveStartupEntries;
  RemovePathEntry;
  RegDeleteKeyIncludingSubkeys(HKLM, StateKey);
end;
