; Inno Setup 6 script for IsoFoundry-v{VERSION}-x64-Setup.exe (D-044).
; Per-user install by default (no administrator rights needed); the dialog
; offers an all-users install. The payload is compiled into the exe.
;
; Compile: ISCC.exe /DAppVersion=0.1.0 /DSourceDir=<staging> /DOutputDir=<dist> isofoundry.iss

#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef SourceDir
  #error SourceDir is required
#endif
#ifndef OutputDir
  #define OutputDir "."
#endif

[Setup]
AppId={{5B1C6E0A-7F2D-4E3B-9A61-2C8D4F0E7B93}
AppName=IsoFoundry
AppVersion={#AppVersion}
AppVerName=IsoFoundry {#AppVersion}
AppPublisher=Sean Shaffer
AppPublisherURL=https://github.com/SeanShaffer88/IsoFoundry
AppSupportURL=https://github.com/SeanShaffer88/IsoFoundry/issues
DefaultDirName={autopf}\IsoFoundry
DefaultGroupName=IsoFoundry
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19045
LicenseFile={#SourceDir}\LICENSE
SetupIconFile={#SourceDir}\icon.ico
UninstallDisplayIcon={app}\isofoundry.exe
OutputDir={#OutputDir}
OutputBaseFilename=IsoFoundry-v{#AppVersion}-x64-Setup
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
ChangesEnvironment=yes
CloseApplications=yes

[Tasks]
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked
Name: "addtopath"; Description: "Add IsoFoundry to PATH so ""isofoundry"" works in any terminal"; Flags: unchecked

[Files]
Source: "{#SourceDir}\isofoundry.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\isofoundry.com"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\isofoundry-helper.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\LICENSE"; DestDir: "{app}"
Source: "{#SourceDir}\README.md"; DestDir: "{app}"
Source: "{#SourceDir}\CHANGELOG.md"; DestDir: "{app}"

[Icons]
Name: "{autoprograms}\IsoFoundry"; Filename: "{app}\isofoundry.exe"
Name: "{autodesktop}\IsoFoundry"; Filename: "{app}\isofoundry.exe"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}')) and not IsAdminInstallMode
Root: HKLM; Subkey: "SYSTEM\CurrentControlSet\Control\Session Manager\Environment"; ValueType: expandsz; ValueName: "Path"; ValueData: "{olddata};{app}"; Tasks: addtopath; Check: NeedsAddPath(ExpandConstant('{app}')) and IsAdminInstallMode

[Run]
Filename: "{app}\isofoundry.exe"; Description: "Start IsoFoundry"; Flags: nowait postinstall skipifsilent

[Code]
function PathKey(var Root: Integer; var Key: String): Boolean;
begin
  if IsAdminInstallMode then begin
    Root := HKEY_LOCAL_MACHINE;
    Key := 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  end else begin
    Root := HKEY_CURRENT_USER;
    Key := 'Environment';
  end;
  Result := True;
end;

function NeedsAddPath(Dir: String): Boolean;
var
  Root: Integer;
  Key, Paths: String;
begin
  PathKey(Root, Key);
  if not RegQueryStringValue(Root, Key, 'Path', Paths) then begin
    Result := True;
    exit;
  end;
  Result := Pos(';' + Uppercase(Dir) + ';', ';' + Uppercase(Paths) + ';') = 0;
end;

procedure RemoveFromPath(Dir: String);
var
  Root, P: Integer;
  Key, Paths: String;
begin
  PathKey(Root, Key);
  if not RegQueryStringValue(Root, Key, 'Path', Paths) then exit;
  Paths := ';' + Paths + ';';
  P := Pos(';' + Uppercase(Dir) + ';', Uppercase(Paths));
  if P = 0 then exit;
  Delete(Paths, P, Length(Dir) + 1);
  Paths := Copy(Paths, 2, Length(Paths) - 2);
  RegWriteExpandStringValue(Root, Key, 'Path', Paths);
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Code: Integer;
begin
  if CurUninstallStep = usUninstall then begin
    RemoveFromPath(ExpandConstant('{app}'));
    if not UninstallSilent and
       (MsgBox('Also delete IsoFoundry settings and logs?', mbConfirmation, MB_YESNO or MB_DEFBUTTON2) = IDYES) then
      Exec(ExpandConstant('{app}\isofoundry.com'), 'clean --yes', '', SW_HIDE, ewWaitUntilTerminated, Code);
  end;
end;
