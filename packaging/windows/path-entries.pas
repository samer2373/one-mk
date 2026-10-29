{ Whole-entry handling of a Windows PATH value (entries separated by ';'),
  and of the one-mk startup value. Included by one-mk.iss ([Code]); the same
  steps are run by scripts/check-windows-path.sh against a list of cases, so
  keep the two in step. Only Pos, Copy, Length and Lowercase are used, and
  strings count from 1, as in Inno Setup's Pascal Script.

  An entry is "the same" only if it is equal to Dir, ignoring case: never a
  prefix or a part of a longer entry (C:\Program Files\one-mk-tools is not
  C:\Program Files\one-mk). }

{ True if Entries has an entry equal to Dir. }
function PathHasEntry(Entries, Dir: string): Boolean;
var
  Rest, Entry: string;
  P: Integer;
begin
  Result := False;
  Rest := Entries + ';';
  Dir := Lowercase(Dir);
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Entry := Copy(Rest, 1, P - 1);
    Rest := Copy(Rest, P + 1, Length(Rest));
    if Lowercase(Entry) = Dir then
      Result := True;
  end;
end;

{ Entries with Dir at the end, unless it is there already. }
function PathWithEntry(Entries, Dir: string): string;
begin
  if PathHasEntry(Entries, Dir) then
    Result := Entries
  else if Entries = '' then
    Result := Dir
  else if Copy(Entries, Length(Entries), 1) = ';' then
    Result := Entries + Dir
  else
    Result := Entries + ';' + Dir;
end;

{ Entries without the entries equal to Dir; every other entry (empty ones
  too) stays as it was, in order. }
function PathWithoutEntry(Entries, Dir: string): string;
var
  Rest, Entry: string;
  P: Integer;
  First: Boolean;
begin
  Result := '';
  First := True;
  Rest := Entries + ';';
  Dir := Lowercase(Dir);
  while Rest <> '' do
  begin
    P := Pos(';', Rest);
    Entry := Copy(Rest, 1, P - 1);
    Rest := Copy(Rest, P + 1, Length(Rest));
    if Lowercase(Entry) <> Dir then
    begin
      if not First then
        Result := Result + ';';
      Result := Result + Entry;
      First := False;
    end;
  end;
end;

{ True if a Run value (`"C:\Program Files\one-mk\one-mk-tray.exe" --start-daemon`)
  starts a program inside Dir, so it is ours and not the user's own. }
function RunValueIsOurs(Data, Dir: string): Boolean;
begin
  if Copy(Data, 1, 1) = '"' then
    Data := Copy(Data, 2, Length(Data));
  Result := Lowercase(Copy(Data, 1, Length(Dir) + 1)) = Lowercase(Dir) + '\';
end;
