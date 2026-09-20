#ifndef MyAppVersion
  #define MyAppVersion "0.1.0"
#endif

#define MyAppName "ctrl-ime-key"
#define MyAppPublisher "SI"
#define MyAppURL "https://github.com/idas709/ctrl-ime-key"
#define MyAppExeName "ctrl-ime-key.exe"

[Setup]
AppId={{DFF7BB95-44B8-4B6B-9366-97177685D2FD}
AppName={#MyAppName}
AppVersion={#MyAppVersion}
AppPublisher={#MyAppPublisher}
AppPublisherURL={#MyAppURL}
AppSupportURL={#MyAppURL}/issues
AppUpdatesURL={#MyAppURL}/releases
DefaultDirName={autopf}\{#MyAppName}
DefaultGroupName={#MyAppName}
DisableProgramGroupPage=yes
LicenseFile=..\LICENSE
OutputDir=..\dist
OutputBaseFilename={#MyAppName}-{#MyAppVersion}-setup-x64
SetupIconFile=..\assets\app.ico
UninstallDisplayIcon={app}\{#MyAppExeName}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
UsePreviousPrivileges=yes
CloseApplications=yes
RestartApplications=no
ChangesEnvironment=no

[Languages]
Name: "japanese"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "startup"; Description: "Start automatically when signing in to Windows"; GroupDescription: "Additional options:"; Flags: checkedonce

[Files]
Source: "..\target\release\{#MyAppExeName}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{group}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"
Name: "{group}\Uninstall"; Filename: "{uninstallexe}"
Name: "{autostartup}\{#MyAppName}"; Filename: "{app}\{#MyAppExeName}"; Tasks: startup

[Run]
Filename: "{app}\{#MyAppExeName}"; Description: "{#MyAppName} を起動する"; Flags: nowait postinstall skipifsilent
