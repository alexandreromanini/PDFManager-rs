; Inno Setup 6 - appelé par la CI : ISCC /DAppVersion=x.y.z packaging\installer.iss
; Fichier en UTF-8 avec BOM. Installé à côté de la version Python (autre nom, autre AppId).
#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
; Modèle de l'assistant IA : mêmes valeurs que src/assistant.rs ; même dossier que la
; version Python (un seul téléchargement de 2,5 Go pour les deux).
#define AiFile "Qwen3-4B-Instruct-2507-Q4_K_M.gguf"
#define AiUrl "https://huggingface.co/unsloth/Qwen3-4B-Instruct-2507-GGUF/resolve/main/" + AiFile
#define AiSha256 "3605803b982cb64aead44f6c1b2ae36e3acdb41d8e46c8a94c6533bc4c67e597"
#define AiDir "{localappdata}\PDFManager\ai"

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
Name: "ai"; Description: "Assistant IA hors ligne : résumer un PDF, lui poser des questions (téléchargement de 2,5 Go, 8 Go de mémoire conseillés)"; Flags: unchecked

[Files]
Source: "..\target\release\pdfmanager.exe"; DestDir: "{app}"; Flags: ignoreversion
; Bibliothèques C++ de Windows (llama.cpp) : déploiement local, rien à installer à part.
Source: "..\dist\crt\*.dll"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist
; Modèles de reconnaissance de texte (Tesseract, licence Apache 2.0).
Source: "..\resources\tessdata\*.traineddata"; DestDir: "{app}\tessdata"; Flags: ignoreversion
Source: "{tmp}\{#AiFile}"; DestDir: "{#AiDir}"; Flags: external skipifsourcedoesntexist ignoreversion; Tasks: ai

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

[Code]
var
  AiPage: TDownloadWizardPage;

procedure InitializeWizard;
begin
  AiPage := CreateDownloadPage('Assistant IA', 'Téléchargement du modèle (2,5 Go, une seule fois)…', nil);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID = wpReady) and WizardIsTaskSelected('ai')
     and not FileExists(ExpandConstant('{#AiDir}\{#AiFile}')) then begin
    AiPage.Clear;
    AiPage.Add('{#AiUrl}', '{#AiFile}', '{#AiSha256}');
    AiPage.Show;
    try
      try
        AiPage.Download;
      except
        // L'application s'installe quand même ; le modèle se télécharge aussi depuis elle.
        if not AiPage.AbortedByUser then
          SuppressibleMsgBox('Le modèle de l''assistant IA n''a pas pu être téléchargé : '
            + GetExceptionMessage + #13#10#13#10 + 'PDFManager s''installe sans lui ; '
            + 'il pourra être téléchargé depuis le panneau Assistant de l''application.',
            mbInformation, MB_OK, IDOK);
      end;
    finally
      AiPage.Hide;
    end;
  end;
end;
