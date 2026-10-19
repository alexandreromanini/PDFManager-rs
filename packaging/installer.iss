; Inno Setup 6 - appelé par la CI : ISCC /DAppVersion=x.y.z packaging\installer.iss
; Fichier en UTF-8 avec BOM. Installé à côté de la version Python (autre nom, autre AppId).
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif

[Setup]
AppId={{5C2B7E14-9D3A-4F61-8B0E-2A7C4D9E1F35}
AppName=PDFManager (Rust)
AppVersion={#AppVersion}
AppPublisher=PDFManager
DefaultDirName={autopf}\PDFManager-rs
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
OutputDir=..\dist
OutputBaseFilename=PDFManager-rs-{#AppVersion}-setup
Compression=lzma2/max
SolidCompression=yes
ChangesAssociations=yes
UninstallDisplayIcon={app}\pdfmanager.exe
SetupIconFile=pdfmanager.ico
WizardStyle=modern

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked
Name: "fileassoc"; Description: "Associer les fichiers .pdf (menu Ouvrir avec)"; Flags: unchecked

[Files]
Source: "..\target\release\pdfmanager.exe"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\PDFManager (Rust)"; Filename: "{app}\pdfmanager.exe"
Name: "{autodesktop}\PDFManager (Rust)"; Filename: "{app}\pdfmanager.exe"; Tasks: desktopicon

[Registry]
Root: HKA; Subkey: "Software\Classes\.pdf\OpenWithProgids"; ValueType: string; ValueName: "PDFManager-rs.Document"; ValueData: ""; Flags: uninsdeletevalue; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\PDFManager-rs.Document"; ValueType: string; ValueName: ""; ValueData: "Document PDF"; Flags: uninsdeletekey; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\PDFManager-rs.Document\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: "{app}\pdfmanager.exe,0"; Tasks: fileassoc
Root: HKA; Subkey: "Software\Classes\PDFManager-rs.Document\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\pdfmanager.exe"" ""%1"""; Tasks: fileassoc

[Run]
Filename: "{app}\pdfmanager.exe"; Description: "{cm:LaunchProgram,PDFManager (Rust)}"; Flags: nowait postinstall skipifsilent
; Mise à jour depuis l'application (installation silencieuse) : relance automatique.
Filename: "{app}\pdfmanager.exe"; Flags: nowait postinstall skipifnotsilent
