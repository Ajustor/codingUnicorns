# 🦄 Coding Unicorns

Un IDE léger et performant construit en Rust, inspiré de VSCode.  
Consommation RAM cible : **30–80 MB** contre 300–500 MB pour VSCode.

**Philosophie** : l'éditeur est volontairement nu. Aucun langage n'est intégré en dur — la coloration syntaxique, le LSP et les outils de développement sont fournis par des **extensions installables**.

---

## Fonctionnalités

### Éditeur
- ✏️ Éditeur de texte avec undo/redo, sélection, word wrap
- 🔀 Split editor — vue côte à côte : bouton à droite des onglets, commande « Split Editor: Toggle » ou Ctrl+\\
- 🔢 Multi-curseurs (Ctrl+D, Ctrl+Shift+L, Alt+↑/↓, Ctrl+Clic)
- 📋 Multi-cursor paste — colle une ligne par curseur quand le nombre correspond
- 🔍 Highlight des occurrences du mot sélectionné
- 🔒 Auto-close brackets/quotes avec surround et pair-delete
- 📂 Code folding par indentation
- 🔗 Bracket matching

### Navigation
- 🔍 Command palette (Ctrl+P fichiers, Ctrl+Shift+P commandes)
- 🗂️ Arbre de fichiers avec icônes, menu contextuel, renommage inline
- 🏷️ Breadcrumbs avec symbole courant
- 🧭 Go to Definition (F12) + navigation back/forward (Alt+←/→)
- 📍 Go to Line (Ctrl+G)
- 🔎 Recherche workspace (Ctrl+Shift+F, pré-remplie avec la sélection)
- 🔎 Find & Replace dans le fichier (Ctrl+F / Ctrl+H) avec regex

### Git
- 🌿 Branche courante + indicateur ahead/behind
- 📊 Arbre des branches (local/remote) avec graph des commits
- ➕ Stage/unstage par fichier ou en bloc
- 💬 Commit, Push, Pull depuis l'UI
- 🔀 3-panel merge tool (Ours | Result | Theirs) pour les conflits
- 📝 Git blame par ligne
- 🎨 Diff gutter (ajouts/modifications/suppressions)
- 🌱 Gestion des branches (boutons ou clic droit) : checkout, merge, create (depuis HEAD, une branche ou un commit du graphe), rename, delete avec confirmation

### LSP (Language Server Protocol)
- 💡 Hover, complétion, go-to-definition, find references
- ✏️ Rename symbol, code actions, signature help
- 📐 Format document
- ⚠️ Diagnostics inline (erreurs, warnings)
- 🔄 Restart LSP via command palette

### Terminal
- 🖥️ Terminal intégré (PTY réel, multi-onglets, 256 couleurs)
- ⌨️ Historique shell, complétion Tab, Ctrl+C/D

### Debug (DAP)
- 🐛 Breakpoints (clic dans la marge ou `F9`), step over/into/out, call stack, variables, expressions surveillées
- ▶️ Configurations de lancement (`launch.toml`, import de `.vscode/launch.json`)
- 🔨 `preLaunchTask` exécutée avant le lancement (tâches `process`, `shell` et `npm` de `.vscode/tasks.json`, avec leurs `dependsOn`)
- 🧰 Le débogueur d'un langage vient de son module (section `[debugger]` du manifeste), l'IDE n'en embarque aucun. Modules officiels : **C#** (netcoredbg), **TypeScript / JavaScript / React / Vue / Svelte / HTML** (vscode-js-debug, Node ou Chrome/Edge), **PowerShell** (PowerShell Editor Services) — téléchargés au premier lancement —, **Python** (debugpy), **Go** (Delve) et **Rust** (lldb-dap)

### Extensions
- 🌐 Registre en ligne : parcourir, installer et mettre à jour les modules officiels depuis le panneau Extensions (binaires précompilés vérifiés par SHA-256)
- 🧩 Modules natifs (`cdylib`) décrits par un `manifest.toml` : coloration, hover et serveur LSP
- 📦 Installation aussi depuis git, une archive ZIP, un dossier local ou un workspace Cargo
- 🔧 Dépendances du serveur de langage (npm, pip, cargo, go, dotnet) installées automatiquement, et retirables à la désinstallation
- 🪟 Interfaces fournies par les modules : panneaux dans la barre latérale et pages en onglet (ex. gestion des images Docker)

### Configuration
- ⚙️ Thèmes personnalisables (Dark, Monokai, Solarized Dark, One Dark + custom RGB)
- ⌨️ **43 raccourcis clavier configurables** dans Settings → Keybindings
- 💾 Auto-save optionnel
- 🔄 Mises à jour automatiques via GitHub Pages (vérification au démarrage, checksum SHA-256)

---

## Prérequis

### Linux / macOS
- [Rust](https://rustup.rs/) 1.75+
- Bibliothèques système (Linux) :
  ```bash
  # Ubuntu / Debian
  sudo apt install libgtk-3-dev libxcb-render0-dev libxcb-shape0-dev \
                   libxcb-xfixes0-dev libxkbcommon-dev libssl-dev
  # Fedora
  sudo dnf install gtk3-devel libxcb-devel xkeyboard-config-devel openssl-devel
  ```

### Windows
- [Rust](https://rustup.rs/) 1.75+
- [Visual Studio Build Tools](https://visualstudio.microsoft.com/fr/visual-cpp-build-tools/) avec le composant C++

---

## Installation

Depuis la [page de téléchargement](https://ajustor.github.io/codingUnicorns/fr/) :

- **macOS** (Apple Silicon) : ouvrez `coding-unicorns-macos-arm64.dmg` et glissez **Coding Unicorns** dans **Applications**. L'app est signée ad hoc, sans notarisation Apple : au premier lancement, clic droit → **Ouvrir** (ou Réglages Système → Confidentialité et sécurité → **Ouvrir quand même**).
- **Linux** (x86_64) : `chmod +x coding-unicorns-linux-x64.AppImage` puis lancez-la (FUSE requis : paquet `libfuse2` ou `fuse`). Un intégrateur comme AppImageLauncher ou Gear Lever l'ajoute au menu des applications.
- **Windows** : installateur `coding-unicorns-setup.msi`.

Les binaires seuls (`coding-unicorns-linux-x64`, `coding-unicorns-macos-arm64`, `.exe`) restent publiés pour un usage en ligne de commande.

Les paquets sont produits par `scripts/build-macos-app.sh` (`.app` + `.dmg`, à lancer sur macOS) et `scripts/build-appimage.sh` (Linux), à partir du binaire release et des fichiers de `packaging/`.

### Depuis les sources

```bash
git clone https://github.com/votre-utilisateur/codingUnicorns
cd codingUnicorns
cargo build --release
```

Le binaire se trouve dans `target/release/coding-unicorns` (ou `.exe` sur Windows).

```bash
cargo run --release
```

---

## Utilisation

### Premier lancement

1. Lancez l'application
2. **File → Open Folder…** (ou `Ctrl+O`) pour ouvrir un projet
3. Installez des extensions de langage pour obtenir la coloration et le LSP : panneau **Extensions** → **🌐 BROWSE REGISTRY** → **Install**

### Depuis le terminal

La commande `cu` ouvre un dossier ou un fichier, comme `code` pour VSCode :

```bash
cu .              # ouvre le dossier courant
cu chemin/fichier # ouvre un fichier ou un dossier
```

Elle rend la main tout de suite : l'IDE continue en arrière-plan, détaché du terminal. Sur Linux et macOS, `--wait` le garde au premier plan. Si l'IDE est déjà ouvert, le chemin est envoyé à la fenêtre existante. Le chemin est toujours converti en chemin absolu, donc `${workspaceFolder}` vaut le dossier complet dans les configurations de lancement.

L'installateur MSI ajoute `cu` au `PATH` sous Windows ; ailleurs, lancez `scripts/install-cli.sh` (il trouve aussi l'app macOS installée dans `/Applications`) (ou `scripts/install-cli.ps1` pour une installation via `cargo install` sous Windows).

### Interface

```
[Barre d'activité] [Sidebar]    [Éditeur gauche] | [Éditeur droit]
                                 [Terminal]
```

| Zone | Description |
|------|-------------|
| **Barre d'activité** | Explorer, Recherche, Git, Extensions, Run, Outline, Debug |
| **Sidebar** | Arbre de fichiers, recherche workspace, git panel, extensions |
| **Éditeur** | Zone principale avec onglets, split possible (bouton à droite des onglets, palette ou Ctrl+\\) |
| **Terminal** | Terminal intégré multi-onglets |
| **Status bar** | Branche git, type fichier, position curseur, statut LSP |

### Raccourcis clavier

#### Général

| Raccourci | Action |
|-----------|--------|
| `Ctrl+O` | Ouvrir un dossier |
| `Ctrl+Shift+O` | Ouvrir un fichier |
| `Ctrl+N` | Nouveau fichier |
| `Ctrl+S` | Sauvegarder |
| `Ctrl+W` | Fermer l'onglet |
| `Ctrl+P` | Command palette (fichiers) |
| `Ctrl+Shift+P` | Command palette (commandes) |
| `Ctrl+B` | Toggle sidebar |
| `Ctrl+\`` | Toggle terminal |
| `Ctrl+\\` | Toggle split editor |
| `Ctrl+,` | Paramètres |
| `F1` | Aide raccourcis |

#### Éditeur

| Raccourci | Action |
|-----------|--------|
| `Ctrl+F` | Rechercher |
| `Ctrl+H` | Rechercher & Remplacer |
| `Ctrl+G` | Aller à la ligne |
| `Ctrl+Z` | Annuler |
| `Ctrl+Shift+Z` | Rétablir |
| `Ctrl+A` | Tout sélectionner |
| `Ctrl+/` | Toggle commentaire |
| `Ctrl+Shift+K` | Supprimer la ligne |
| `Ctrl+Shift+D` | Dupliquer la ligne |
| `Ctrl+Enter` | Insérer ligne en-dessous |
| `Ctrl+Shift+Enter` | Insérer ligne au-dessus |
| `Alt+↑/↓` | Déplacer la ligne |
| `Ctrl+]` / `Ctrl+[` | Indenter / Désindenter |
| `Ctrl+Space` | Complétion |

#### Multi-curseurs

| Raccourci | Action |
|-----------|--------|
| `Ctrl+Clic` | Ajouter un curseur |
| `Ctrl+D` | Sélectionner la prochaine occurrence |
| `Ctrl+Shift+L` | Sélectionner toutes les occurrences |
| `Ctrl+Alt+↑/↓` | Ajouter curseur au-dessus/en-dessous |

#### Navigation

| Raccourci | Action |
|-----------|--------|
| `F12` | Aller à la définition |
| `Alt+←` | Naviguer en arrière |
| `Alt+→` | Naviguer en avant |
| `Shift+F12` | Trouver les références |

#### Code

| Raccourci | Action |
|-----------|--------|
| `F2` | Renommer le symbole |
| `Ctrl+.` | Actions de code |
| `Shift+Alt+F` | Formater le document |
| `Ctrl+Alt+B` | Toggle git blame |

#### Debug

| Raccourci | Action |
|-----------|--------|
| `F5` | Démarrer / Continuer |
| `F9` | Toggle breakpoint |
| `F10` | Step over |
| `F11` | Step into |
| `Shift+F11` | Step out |

> Tous les raccourcis sont configurables dans **Settings → Keybindings**.

---

## Extensions de langage

L'éditeur ne contient aucun support de langage intégré. La coloration syntaxique, le hover et le LSP viennent d'extensions : des modules natifs (`cdylib`) chargés au démarrage.

### Extensions officielles

Les modules officiels sont publiés par le dépôt [`coding-unicorns-modules`](https://github.com/Ajustor/coding-unicorns-modules). La liste complète, avec les versions publiées, est sur la **[page des extensions](https://ajustor.github.io/coding-unicorns-modules/)** (index machine : [`registry.json`](https://ajustor.github.io/coding-unicorns-modules/registry.json)).

| Extension | Fichiers | Serveur LSP | Débogueur | Dépendances installées |
|-----------|----------|-------------|-----------|------------------------|
| **rust-lang** | `.rs` | rust-analyzer | lldb-dap (LLVM) | — (rust-analyzer via rustup) |
| **javascript-lang** | `.js` `.mjs` | typescript-language-server | vscode-js-debug | npm |
| **typescript-lang** | `.ts` | typescript-language-server | vscode-js-debug | npm |
| **react-lang** | `.jsx` `.tsx` | typescript-language-server | vscode-js-debug | npm |
| **python-lang** | `.py` `.pyw` | pylsp | debugpy | pip |
| **go-lang** | `.go` | gopls | Delve | go |
| **vue-lang** | `.vue` | vue-language-server | vscode-js-debug | npm |
| **svelte-lang** | `.svelte` | svelte-language-server | vscode-js-debug | npm |
| **html-lang** | `.html` `.htm` | vscode-html-language-server | vscode-js-debug | npm |
| **xml-lang** | `.xml` `.xsl` `.xsd` `.svg` `.xhtml` | lemminx | — | — (à installer) |
| **toml-lang** | `.toml` | taplo | — | — (à installer) |
| **csharp-lang** | `.cs` `.csx` | csharp-ls | netcoredbg | dotnet (SDK .NET requis) |
| **powershell-lang** | `.ps1` `.psm1` `.psd1` | — | PowerShell Editor Services | — |
| **spd-lang** | `.spd` | speedster-language-server | — | — (à installer) |
| **json-lang** | `.json` `.jsonc` `.json5` `.geojson` `.webmanifest` | vscode-json-language-server | — | npm |
| **docker-lang** | `Dockerfile` `Containerfile` `Dockerfile.*`, `compose.yaml` `docker-compose.yml` (et `*.override.yml`…) | docker-langserver, docker-compose-langserver | — | npm |

**docker-lang** ajoute aussi un panneau **Docker** dans la barre d'activité : liste des images locales, **Pull**, **Run** (dans un terminal), **Remove**, **Prune** des images sans tag, et **Full view** qui ouvre le tableau complet dans un onglet. **Containers & logs** ouvre en onglet la liste des conteneurs (**Start**, **Stop**, **Restart**, **Remove**, **Shell**) et leurs **Logs**, suivis en direct. Il faut le CLI `docker` dans le `PATH`.

Des binaires précompilés existent pour **Windows x86_64**, **Linux x86_64** et **macOS Apple Silicon**. Sur une autre plateforme, le registre affiche « Not available for this platform » : il faut alors compiler depuis les sources (voir ci-dessous).

### Installer des extensions

Tout se passe dans le panneau **Extensions** (icône de pièce de puzzle dans la barre d'activité).

#### Depuis le registre (recommandé)

1. Section **🌐 BROWSE REGISTRY** (ouverte par défaut) ; **Search modules…** filtre par nom, langage ou description
2. **Install** sur le module voulu

L'archive précompilée pour votre plateforme est téléchargée et vérifiée (taille et SHA-256 ; une empreinte absente ou différente refuse l'installation), puis ses dépendances sont installées. Aucune toolchain Rust n'est nécessaire. Un échec sur une dépendance n'annule pas l'installation : il est signalé par « Installed with warnings ».

Le registre se configure dans **Settings → Extensions → Module registry URL** (`[extensions] registry_url` dans `config.toml`) : **Reset** rétablit l'adresse officielle, un champ vide désactive le registre. L'ancienne adresse `writing-unicorns-modules` est migrée automatiquement.

#### Autres sources

Ces méthodes compilent le module sur votre machine et demandent donc une toolchain Rust (sauf le ZIP) :

| Section du panneau | Usage |
|--------------------|-------|
| **INSTALL FROM GIT** | URL d'un dépôt avec un `manifest.toml` à la racine → **Install** (clone puis `cargo build --release`) |
| **📁 Load from local folder** | **Browse folder…** vers un dossier contenant un `manifest.toml` (réutilise `target/release` s'il est déjà compilé) |
| **📦 INSTALL FROM ZIP** | **Browse ZIP…** vers une archive de dossiers `<module>/manifest.toml` + bibliothèque, puis choix des modules. Les dépendances ne sont **pas** installées |
| **⚙ BUILD FROM SOURCES** | Chemin d'un workspace Cargo local → **▶ Select & Install**, puis choix des modules |
| **📦 INSTALL GROUP FROM GIT** | URL d'un dépôt workspace (ex. `https://github.com/Ajustor/coding-unicorns-modules`) → **Install All** installe chaque membre qui a un `manifest.toml` |

#### Mettre à jour et désinstaller

- En haut du panneau, **⟳ Check for updates** ; un module plus récent affiche **Update** (ou **Update to X** dans le registre), et **⬆ Update all**, à côté, les met tous à jour d'un coup.
  - Modules du registre : comparés à l'index en ligne.
  - Modules installés depuis un dossier ou un workspace : comparés au `manifest.toml` source.
  - Modules installés depuis git ou un ZIP : pas de vérification automatique, il faut les réinstaller.
- **🗑 Uninstall**, dans la section **INSTALLED** comme dans le registre pour un module installé, décharge le module puis propose **Module only** ou **Module + dependencies** (désinstalle les paquets npm, pip, cargo et dotnet déclarés ; les outils Go sont conservés).

Les extensions sont installées dans :

| OS | Dossier |
|----|---------|
| Linux | `~/.config/coding-unicorns/extensions/<id>/` |
| macOS | `~/Library/Application Support/coding-unicorns/extensions/<id>/` |
| Windows | `%APPDATA%\coding-unicorns\extensions\<id>\` |

### Créer une extension

Chaque extension est un crate Rust compilé en `cdylib`, avec un `manifest.toml` à sa racine :

```toml
[extension]
id = "acme.mylang"          # lettres, chiffres, . _ - ; sert de nom de dossier
name = "MyLang"
version = "0.1.0"           # comparé en major.minor.patch pour les mises à jour
description = "Support pour MyLang"
author = "Acme"             # optionnel
repository = "https://github.com/acme/mylang"  # optionnel

[capabilities]
languages = ["ml", "mli"]   # extensions de fichiers, sans le point
lsp_server = "mylang-lsp"   # optionnel : binaire du serveur LSP
lsp_args = ["--stdio"]
language_ids = { mli = "mylang" }  # optionnel : languageId LSP quand il diffère de l'extension
lsp_init_options = { provideFormatter = true }  # optionnel : initializationOptions du serveur
file_names = { "MyLangfile" = "ml" }  # optionnel : fichiers reconnus par leur nom (motifs `*`, sans casse)

[capabilities.lsp_servers.mli]  # optionnel : un autre serveur pour un des langages
command = "mylang-iface-lsp"
args = ["--stdio"]
init_options = {}               # optionnel, remplace lsp_init_options pour ce langage

[dependencies]              # optionnel, installé avec le module
npm = ["mylang-lsp"]        # npm install -g
pip = []                    # pip3 install
cargo = []                  # cargo install
go = []                     # go install
dotnet = ["some-tool@1.2.3"]  # dotnet tool update --global (version épinglable)

[debugger]                  # optionnel : adaptateur de debug (DAP)
types = ["mylang"]          # `type` des configurations de launch.json servies
command = ["mylang-dap"]    # binaire (ou candidats dans l'ordre), cherché dans le PATH
args = ["--stdio"]          # ${debuggerDir} = dossier du téléchargement, ${port} = port TCP
transport = "stdio"         # ou "tcp" : l'adaptateur écoute sur 127.0.0.1:${port}
type_map = { mylang-old = "mylang" }        # optionnel : renomme le `type` des configurations
install_hint = "cargo install mylang-dap"   # affiché si introuvable

[debugger.default_launch]   # optionnel : arguments de `launch` sans configuration
program = "${file}"

[debugger.download]         # optionnel : archive décompressée dans ${debuggerDir}
binary = "mylang-dap/mylang-dap"            # fichier qui atteste l'installation (sans .exe)
[debugger.download.urls]    # .zip ou .tar.gz par plateforme
windows-x86_64 = "https://example.com/mylang-dap-win64.zip"
linux-x86_64 = "https://example.com/mylang-dap-linux.tar.gz"

[[panels]]                  # optionnel : interfaces du module (voir plus bas)
id = "mylang.tools"         # unique, passé à ui_view_ffi / ui_event_ffi
title = "MyLang"
icon = "cube"               # nom Phosphor (cube, package, container, database, terminal…) ou texte
location = "sidebar"        # "sidebar" (barre d'activité) ou "page" (onglet)
```

Le crate exporte des fonctions C :

| Symbole | Rôle |
|---------|------|
| `language_id()` | **Requis.** Nom du langage |
| `file_extensions()` | **Requis.** Extensions supportées, séparées par des virgules (ex. `"ml,mli"`) |
| `tokenize_line_ffi(line)` | JSON `[{"text","kind"}]` des tokens d'une ligne (`keyword`, `type`, `string`, `comment`, `number`, `function`, `macro`, `property`, `operator`, `class`) |
| `tokenize_document_ffi(text)` | Tokens du document entier (un tableau par ligne), pour les constructions multi-lignes |
| `tokenize_document_tsx_ffi(text)` | Variante utilisée pour `.tsx` / `.jsx` |
| `reset_tokenizer()` | Réinitialise l'état du tokenizer |
| `hover_info_ffi(word, content)` | Texte de survol |
| `tokenize_line_lang_ffi(lang, line)`, `tokenize_document_lang_ffi(lang, text)`, `hover_info_lang_ffi(lang, word, content)` | Variantes qui reçoivent le langage du fichier, utilisées en priorité : un module qui gère plusieurs langages (Dockerfile et Compose) choisit ainsi son tokenizer |
| `ui_view_ffi(panel_id)` | Vue JSON d'un panneau déclaré dans `[[panels]]` |
| `ui_event_ffi(panel_id, event)` | Reçoit un événement JSON du panneau, renvoie un tableau JSON d'actions (ou null) |
| `free_string(ptr)` | Libère **toute** chaîne renvoyée par les fonctions ci-dessus |

Le serveur LSP et le débogueur sont déclarés dans le manifeste, pas exportés : un module peut fournir un débogueur sans bibliothèque. Au lancement (F5), l'IDE prend le module dont `types` contient le `type` de la configuration de debug, sinon celui du langage du fichier ouvert ; sans `default_launch`, une configuration dans `launch.toml` ou `.vscode/launch.json` est nécessaire. L'archive est téléchargée au premier lancement quand `args` utilise `${debuggerDir}`, ou quand `command` n'est pas dans le `PATH` (le binaire téléchargé sert alors de commande). En TCP, les sessions enfants demandées par l'adaptateur (`startDebugging`, utilisé par vscode-js-debug) sont ouvertes automatiquement. Les [modules officiels](https://github.com/Ajustor/coding-unicorns-modules) sont les meilleurs exemples à copier.

#### Panneaux et pages

Un module ne dessine rien lui-même : `ui_view_ffi` renvoie une vue JSON que l'IDE affiche, et les actions de l'utilisateur lui reviennent par `ui_event_ffi`. La vue est redemandée après chaque événement et toutes les `poll_ms` millisecondes tant que le panneau est visible (1 s par défaut) : un module lance ses tâches longues sur ses propres threads et en montre l'avancement dans la vue suivante.

```json
{ "poll_ms": 3000,
  "actions": [{ "type": "toast", "text": "Image téléchargée" }],
  "children": [
    { "type": "heading", "text": "Images" },
    { "type": "row", "children": [
        { "type": "input", "id": "image", "hint": "nginx:latest", "revision": 0, "submit": "pull" },
        { "type": "button", "id": "pull", "label": "Pull", "icon": "download", "style": "primary" } ] },
    { "type": "list", "empty": "Aucune image", "items": [
        { "id": "a1b2c3", "title": "nginx:latest", "subtitle": "187MB", "actions": [
            { "id": "remove", "label": "Remove", "style": "danger", "confirm": "Supprimer nginx:latest ?" } ] } ] } ] }
```

| Élément | Champs |
|---------|--------|
| `heading`, `text` | `text` ; `style` de `text` : `muted`, `strong`, `code`, `error`, `warning`, `success` |
| `button` | `id`, `label`, `icon`, `style` (`primary`, `danger`), `enabled`, `tooltip`, `confirm` (question posée avant l'envoi) |
| `input` | `id`, `hint`, `value` ; le texte reprend `value` quand `revision` change ; `submit` = bouton cliqué par Entrée |
| `checkbox` | `id`, `label`, `checked` |
| `row`, `group`, `collapsing` | `children` ; `title` pour `group` et `collapsing` (avec `id` et `open`) |
| `list` | `items` : `id`, `title`, `subtitle`, `detail`, `actions` (boutons) ; `empty` |
| `table` | `columns`, `rows` : `id`, `cells`, `actions` ; `empty` |
| `log` | `text` (chasse fixe, sélectionnable, défile jusqu'à la fin), `height` |
| `separator`, `spinner` (`text`), `space` (`size`) | — |

Les éléments inconnus sont ignorés. Événements : `{"type":"click","id":…,"row":…}` (`row` = `id` de l'entrée de liste ou de tableau), `{"type":"submit","id":…,"value":…}`, `{"type":"toggle","id":…,"checked":…}` ; chacun porte `inputs`, le texte de tous les champs du panneau. Actions, renvoyées par `ui_event_ffi` ou dans `actions` d'une vue : `toast` (`text`), `terminal` (`command`, lancée dans un nouveau terminal), `open_panel` (`panel` : ouvre une page en onglet ou sélectionne un panneau latéral), `open_url` (`url`), `open_file` (`path`, relatif au workspace).

---

## Configuration

Le fichier de configuration est stocké dans :

| OS | Chemin |
|----|--------|
| Linux | `~/.config/coding-unicorns/config.toml` |
| macOS | `~/Library/Application Support/coding-unicorns/config.toml` |
| Windows | `%APPDATA%\coding-unicorns\config.toml` |

### Mises à jour

Au démarrage (builds release uniquement), l'IDE lit le manifeste [`latest.json`](https://ajustor.github.io/codingUnicorns/latest.json) publié sur GitHub Pages par le workflow de release (le dépôt étant privé, l'API des releases n'est pas accessible aux clients). Si une version plus récente existe, une fenêtre propose de l'installer ; le fichier téléchargé est vérifié avec le SHA-256 du manifeste.

- **Binaire portable** (Linux, macOS, `.exe` Windows) et **app macOS** : l'exécutable est remplacé sur place (dans `Coding Unicorns.app/Contents/MacOS/` pour l'app), puis l'IDE redémarre.
- **AppImage** (Linux) : le fichier `.AppImage` lancé (`$APPIMAGE`) est remplacé par celui de la nouvelle version, puis l'IDE redémarre.
- **Installation MSI** (Windows, dans `Program Files`) : le `.msi` est téléchargé et lancé via `msiexec` à la fermeture de l'IDE.

Vérification manuelle : palette de commandes → **Check for Updates**. Désactivable dans Settings → Updates (`check_updates = false`).

Pour publier : mettre à jour `version` dans `Cargo.toml`, ajouter la section `## [X.Y.Z] - AAAA-MM-JJ` correspondante dans [`CHANGELOG.md`](CHANGELOG.md) (anglais) **et** [`CHANGELOG.fr.md`](CHANGELOG.fr.md) (français), puis pousser un tag `vX.Y.Z` identique — le workflow de release refuse un tag qui ne correspond pas à `Cargo.toml` ou sans entrée dans l'un des deux changelogs. Le dépôt étant privé, ces changelogs sont ce que voient les utilisateurs : l'anglais sert de texte à la release GitHub et de notes dans la fenêtre de mise à jour (l'interface est en anglais), chacun alimente sa page de téléchargement (version courante + historique). Modifier un changelog ou la page (`pages/`) sur `master` republie le site. Les tags de pré-release (`v1.2.0-beta`) créent une release GitHub mais ne sont pas publiés sur Pages. Seule la dernière version stable est hébergée sur Pages ; page de téléchargement : https://ajustor.github.io/codingUnicorns/ (anglais) et https://ajustor.github.io/codingUnicorns/fr/ (français). Les textes de la page sont dans `pages/i18n/<langue>.json`.

---

## Technologies

| Composant | Technologie |
|-----------|-------------|
| UI | [egui](https://github.com/emilk/egui) / [eframe](https://github.com/emilk/egui/tree/master/crates/eframe) 0.31 |
| Buffer texte | [ropey](https://github.com/cessen/ropey) |
| Syntax highlighting | [tree-sitter](https://tree-sitter.github.io/) (moteur) + extensions |
| Terminal PTY | [portable-pty](https://github.com/wez/wezterm/tree/main/pty) |
| Parsing ANSI | [vte](https://github.com/alacritty/vte) |
| Icônes | [egui-phosphor](https://github.com/lucasmerlin/hello_egui/tree/main/crates/egui-phosphor) |
| Git | [git2](https://github.com/rust-lang/git2-rs) |
| LSP | [lsp-types](https://github.com/gluon-lang/lsp-types) |
| DAP | Debug Adapter Protocol (implémentation custom) |
| Dialogues fichiers | [rfd](https://github.com/PolyMeilex/rfd) |
| Extensions | [libloading](https://github.com/nagisa/rust_libloading) (FFI dynamique) |
| Async | [tokio](https://tokio.rs/) |
| Recherche floue | [fuzzy-matcher](https://github.com/lotabout/fuzzy-matcher) |

---

## Licence

MIT
