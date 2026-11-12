; Windows installer, built by CI with Inno Setup:
;   iscc /DAppVersion=0.1.1 /DReleaseDir=target\x86_64-pc-windows-msvc\release installer\one-mk.iss
; Installs for the current user only (no administrator rights needed).
[Setup]
AppId={{6B1F3C52-0D4A-4E8B-9C7A-1E2D5A4F7B90}
AppName=oneMK
AppVersion={#AppVersion}
AppPublisher=Samer Abu-Sleem
AppPublisherURL=https://github.com/samer2373/one-mk
DefaultDirName={localappdata}\Programs\one-mk
DefaultGroupName=oneMK
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..
OutputBaseFilename=one-mk-x86_64-pc-windows-msvc-setup
SetupIconFile=..\docs\brand\app-icon\windows-linux\onemk.ico
UninstallDisplayIcon={app}\one-mk.ico
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
; The user's Path changes (install and uninstall): tell running programs.
ChangesEnvironment=yes

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "addpath"; Description: "Add one-mk to PATH, to use it from a terminal"

[Files]
Source: "{#ReleaseDir}\one-mk.exe"; DestDir: "{app}"
Source: "{#ReleaseDir}\one-mk-tray.exe"; DestDir: "{app}"
Source: "{#ReleaseDir}\one-mk-app.exe"; DestDir: "{app}"
Source: "{#ReleaseDir}\one-mk-wallpaper.exe"; DestDir: "{app}"
; Licences: oneMK's, every crate built in (THIRD-PARTY-NOTICES), the fonts'.
; The app icon, for the shortcuts and the uninstall entry (the programs carry no icon resource).
Source: "..\docs\brand\app-icon\windows-linux\onemk.ico"; DestDir: "{app}"; DestName: "one-mk.ico"
Source: "..\LICENSE-MIT"; DestDir: "{app}\licenses"
Source: "..\LICENSE-APACHE"; DestDir: "{app}\licenses"
Source: "..\THIRD-PARTY-NOTICES"; DestDir: "{app}\licenses"; DestName: "THIRD-PARTY-NOTICES.txt"
Source: "..\crates\app\fonts\OFL-Inter.txt"; DestDir: "{app}\licenses"
Source: "..\crates\app\fonts\OFL-JetBrainsMono.txt"; DestDir: "{app}\licenses"
Source: "..\crates\app\fonts\OFL-Unbounded.txt"; DestDir: "{app}\licenses"

[Icons]
Name: "{group}\oneMK"; Filename: "{app}\one-mk-app.exe"; IconFilename: "{app}\one-mk.ico"
Name: "{autodesktop}\oneMK"; Filename: "{app}\one-mk-app.exe"; IconFilename: "{app}\one-mk.ico"; Tasks: desktopicon

[Run]
Filename: "{app}\one-mk-app.exe"; Description: "Open oneMK"; Flags: nowait postinstall skipifsilent

; Nothing is run from here on uninstall: [Code] below does it all in a fixed
; order (CurUninstallStepChanged), which [UninstallRun] could not guarantee.

[Code]
{ --- User PATH ---------------------------------------------------------------
  The entries of the user's PATH are split at ';' and compared whole, ignoring
  case and a trailing '\', so C:\Program Files\one-mk-tools is never taken for
  C:\Program Files\one-mk, and an upgrade over an existing install (or running
  the installer twice) never adds a second entry. Uninstalling takes out every
  entry equal to the install folder and leaves all others as they were (empty
  ones too, in order). Only Pos, Copy, Length, Trim and Lowercase are used, and
  strings count from 1. scripts/test-windows-path.py runs a model of this
  logic and checks that these functions still read the way it expects. }

function PathKey(Entry: string): string;
begin
  Result := Lowercase(Trim(Entry));
  while (Length(Result) > 3) and (Copy(Result, Length(Result), 1) = '\') do
    Result := Copy(Result, 1, Length(Result) - 1);
end;

{ True if Entries has an entry equal to Dir. }
function PathHasEntry(Entries, Dir: string): Boolean;
var
  Rest, Entry: string;
  P: Integer;
begin
  Result := False;
  Rest := Entries + ';';
  Dir := PathKey(Dir);
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Entry := Copy(Rest, 1, P - 1);
    Rest := Copy(Rest, P + 1, Length(Rest));
    if (Entry <> '') and (PathKey(Entry) = Dir) then
      Result := True;
  end;
end;

{ Entries with Dir at the end, unless it is there already. }
function PathWithEntry(Entries, Dir: string): string;
begin
  if PathHasEntry(Entries, Dir) then
    Result := Entries
  else if Trim(Entries) = '' then
    Result := Dir
  else if Copy(Entries, Length(Entries), 1) = ';' then
    Result := Entries + Dir
  else
    Result := Entries + ';' + Dir;
end;

{ Entries without the entries equal to Dir; every other entry stays as it was. }
function PathWithoutEntry(Entries, Dir: string): string;
var
  Rest, Entry: string;
  P: Integer;
  First: Boolean;
begin
  Result := '';
  First := True;
  Rest := Entries + ';';
  Dir := PathKey(Dir);
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Entry := Copy(Rest, 1, P - 1);
    Rest := Copy(Rest, P + 1, Length(Rest));
    if (Entry = '') or (PathKey(Entry) <> Dir) then
    begin
      if not First then
        Result := Result + ';';
      Result := Result + Entry;
      First := False;
    end;
  end;
end;

{ The program a Run value starts: "C:\a b\x.exe" --y   or   C:\a\x.exe --y }
function RunValueProgram(Data: string): string;
var
  P: Integer;
begin
  Data := Trim(Data);
  if Copy(Data, 1, 1) = '"' then
  begin
    Data := Copy(Data, 2, Length(Data));
    P := Pos('"', Data);
    if P = 0 then
      Result := ''
    else
      Result := Copy(Data, 1, P - 1);
  end
  else
  begin
    P := Pos(' ', Data);
    if P = 0 then
      Result := Data
    else
      Result := Copy(Data, 1, P - 1);
  end;
end;

{ True if a Run value starts a program that sits directly in Dir: it is ours
  (the rule of `one-mk uninstall`), not the user's own. }
function RunValueIsOurs(Data, Dir: string): Boolean;
var
  ExePath: string;
  I: Integer;
begin
  Result := False;
  ExePath := Lowercase(RunValueProgram(Data));
  StringChangeEx(ExePath, '/', '\', True);
  I := Length(ExePath);
  while (I > 0) and (Copy(ExePath, I, 1) <> '\') do
    I := I - 1;
  if I > 0 then
    Result := PathKey(Copy(ExePath, 1, I - 1)) = PathKey(Dir);
end;

{ --- Setup and uninstall steps ------------------------------------------------ }

const
  EnvironmentKey = 'Environment';
  RunKey = 'Software\Microsoft\Windows\CurrentVersion\Run';

procedure AddAppToPath;
var
  Old, Updated: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Old) then
    Old := '';
  Updated := PathWithEntry(Old, ExpandConstant('{app}'));
  { Written only when it changes, and as REG_EXPAND_SZ like the system does.
    The value is read back unexpanded, so %VARIABLES% in it survive. }
  if Updated <> Old then
    RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Updated);
end;

procedure RemoveAppFromPath;
var
  Old, Updated: string;
begin
  if not RegQueryStringValue(HKCU, EnvironmentKey, 'Path', Old) then
    Exit;
  Updated := PathWithoutEntry(Old, ExpandConstant('{app}'));
  if Updated = Old then
    Exit;
  if Updated = '' then
    RegDeleteValue(HKCU, EnvironmentKey, 'Path')
  else
    RegWriteExpandStringValue(HKCU, EnvironmentKey, 'Path', Updated);
end;

{ Ends the programs that run from the install folder (compared by full path,
  so another copy of oneMK elsewhere is left alone). OnlyWindow: just the tray
  and the window, which could start the daemon again. }
procedure EndRunningPrograms(OnlyWindow: Boolean);
var
  Folder, Names, Script: string;
  Code: Integer;
begin
  Folder := ExpandConstant('{app}');
  StringChangeEx(Folder, '''', '''''', True);
  if OnlyWindow then
    Names := '''one-mk-tray.exe'',''one-mk-app.exe'''
  else
    Names := '''one-mk.exe'',''one-mk-tray.exe'',''one-mk-app.exe'',''one-mk-wallpaper.exe''';
  Script := 'Get-CimInstance Win32_Process | Where-Object { $_.ExecutablePath -and (@(' + Names +
    ') -contains $_.Name.ToLower()) -and $_.ExecutablePath.StartsWith(''' + Folder +
    '\'', [StringComparison]::OrdinalIgnoreCase) } | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }';
  { Hidden; a failure (no PowerShell, an access error) never stops the uninstall. }
  Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -ExecutionPolicy Bypass -Command "' + Script + '"', '', SW_HIDE, ewWaitUntilTerminated, Code);
end;

{ `one-mk uninstall` took the start-at-sign-in entry out already; this is for
  when it could not run. Only a value that starts a program from this folder
  goes. }
procedure RemoveStartAtSignIn;
var
  Data: string;
begin
  if RegQueryStringValue(HKCU, RunKey, 'one-mk', Data) then
    if RunValueIsOurs(Data, ExpandConstant('{app}')) then
      RegDeleteValue(HKCU, RunKey, 'one-mk');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if (CurStep = ssPostInstall) and WizardIsTaskSelected('addpath') then
    AddAppToPath;
end;

{ Uninstalling, as the user who installed (no administrator rights), with all
  the files still there and every program hidden. No exit code is checked: a
  failure here never stops the uninstall.
  1. `one-mk uninstall`: no start at sign-in, no Explorer "Send to" entries
     (they would point at the deleted one-mk.exe), the wallpaper off and the
     person's own back. It asks the running oneMK, so it goes first.
  2. The tray and window end (nothing starts the daemon again), the daemon is
     asked to stop, and whatever still runs from this folder is ended, so no
     file stays in use and is left behind.
  3. The start-at-sign-in entry, if one is still there, and the PATH entry.
  The settings folder (pairings, keys) is kept, so installing again keeps the
  computer paired. }
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
  Exe: string;
begin
  if CurUninstallStep <> usUninstall then
    Exit;
  Exe := ExpandConstant('{app}\one-mk.exe');
  if FileExists(Exe) then
    Exec(Exe, 'uninstall', '', SW_HIDE, ewWaitUntilTerminated, Code);
  EndRunningPrograms(True);
  if FileExists(Exe) then
    Exec(Exe, 'stop', '', SW_HIDE, ewWaitUntilTerminated, Code);
  EndRunningPrograms(False);
  RemoveStartAtSignIn;
  RemoveAppFromPath;
end;
