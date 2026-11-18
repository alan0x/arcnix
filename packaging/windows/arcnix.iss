[Setup]
AppId={{CB89D5EC-A886-4760-8578-99CCCE459EA1}
AppName=Arcnix
AppVersion={#AppVersion}
AppPublisher=Arcnix
AppPublisherURL=https://arcnix.org
VersionInfoVersion={#FileVersion}
DefaultDirName={userpf}\Arcnix
DefaultGroupName=Arcnix
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\..\crates\arcnix\assets\arcnix.ico
UninstallDisplayName=Arcnix
UninstallDisplayIcon={app}\arcnix.exe
RestartApplications=no
Compression=lzma2
SolidCompression=yes
WizardStyle=modern

[Files]
Source: "{#Stage}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Arcnix"; Filename: "{app}\arcnix.exe"; WorkingDir: "{app}"

[UninstallDelete]
Type: files; Name: "{app}\arcnix.exe.previous"
