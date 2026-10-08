# 🦄 Coding Unicorns

Un IDE léger et performant construit en Rust, inspiré de VSCode.  
Consommation RAM cible : **30–80 MB** contre 300–500 MB pour VSCode.

**Philosophie** : l'éditeur est volontairement nu. Aucun langage n'est intégré en dur — la coloration syntaxique, le LSP et les outils de développement sont fournis par des **extensions installables**.

---

## Fonctionnalités

### Éditeur
- ✏️ Éditeur de texte avec undo/redo, sélection, word wrap
- 🔀 Split editor (Ctrl+\\) — vue côte à côte
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
- 🐛 Breakpoints, step over/into/out, call stack, variables
- ▶️ Configurations de lancement (launch.toml)

### Extensions
- 🌐 Registre en ligne : parcourir, installer et mettre à jour les modules officiels depuis le panneau Extensions (binaires précompilés vérifiés par SHA-256)
- 🧩 Modules natifs (`cdylib`) décrits par un `manifest.toml` : coloration, hover et serveur LSP
- 📦 Installation aussi depuis git, une archive ZIP, un dossier local ou un workspace Cargo
- 🔧 Dépendances du serveur de langage (npm, pip, cargo, go, dotnet) installées automatiquement, et retirables à la désinstallation

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

L'installateur MSI ajoute `cu` au `PATH` sous Windows ; ailleurs, lancez `scripts/install-cli.sh` (ou `scripts/install-cli.ps1` pour une installation via `cargo install` sous Windows).

### Interface

```
[Barre d'activité] [Sidebar]    [Éditeur gauche] | [Éditeur droit]
                                 [Terminal]
```

| Zone | Description |
|------|-------------|
| **Barre d'activité** | Explorer, Recherche, Git, Extensions, Run, Outline, Debug |
| **Sidebar** | Arbre de fichiers, recherche workspace, git panel, extensions |
| **Éditeur** | Zone principale avec onglets, split possible (Ctrl+\\) |
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

| Extension | Fichiers | Serveur LSP | Dépendances installées |
|-----------|----------|-------------|------------------------|
| **rust-lang** | `.rs` | rust-analyzer | — (rust-analyzer via rustup) |
| **javascript-lang** | `.js` `.mjs` | typescript-language-server | npm |
| **typescript-lang** | `.ts` | typescript-language-server | npm |
| **react-lang** | `.jsx` `.tsx` | typescript-language-server | npm |
| **python-lang** | `.py` `.pyw` | pylsp | pip |
| **go-lang** | `.go` | gopls | go |
| **vue-lang** | `.vue` | vue-language-server | npm |
| **svelte-lang** | `.svelte` | svelte-language-server | npm |
| **html-lang** | `.html` `.htm` | vscode-html-language-server | npm |
| **xml-lang** | `.xml` `.xsl` `.xsd` `.svg` `.xhtml` | lemminx | — (à installer) |
| **toml-lang** | `.toml` | taplo | — (à installer) |
| **csharp-lang** | `.cs` `.csx` | csharp-ls | dotnet (SDK .NET requis) |
| **powershell-lang** | `.ps1` `.psm1` `.psd1` | — | — |
| **spd-lang** | `.spd` | speedster-language-server | — (à installer) |

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

- Section **INSTALLED** → **⟳ Check for updates** ; un module plus récent affiche **Update** (ou **Update to X** dans le registre).
  - Modules du registre : comparés à l'index en ligne.
  - Modules installés depuis un dossier ou un workspace : comparés au `manifest.toml` source.
  - Modules installés depuis git ou un ZIP : pas de vérification automatique, il faut les réinstaller.
- **Uninstall** décharge le module puis propose **Module only** ou **Module + dependencies** (désinstalle les paquets npm, pip, cargo et dotnet déclarés ; les outils Go sont conservés).

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

[dependencies]              # optionnel, installé avec le module
npm = ["mylang-lsp"]        # npm install -g
pip = []                    # pip3 install
cargo = []                  # cargo install
go = []                     # go install
dotnet = ["some-tool@1.2.3"]  # dotnet tool update --global (version épinglable)
```

Le crate exporte des fonctions C :

| Symbole | Rôle |
|---------|------|
| `language_id()` | **Requis.** Nom du langage |
| `file_extensions()` | **Requis.** Extensions supportées, séparées par des virgules (ex. `"ml,mli"`) |
| `tokenize_line_ffi(line)` | JSON `[{"text","kind"}]` des tokens d'une ligne (`keyword`, `type`, `string`, `comment`, `number`, `function`, `macro`) |
| `tokenize_document_ffi(text)` | Tokens du document entier (un tableau par ligne), pour les constructions multi-lignes |
| `tokenize_document_tsx_ffi(text)` | Variante utilisée pour `.tsx` / `.jsx` |
| `reset_tokenizer()` | Réinitialise l'état du tokenizer |
| `hover_info_ffi(word, content)` | Texte de survol |
| `free_string(ptr)` | Libère **toute** chaîne renvoyée par les fonctions ci-dessus |

Le serveur LSP est déclaré dans le manifeste, pas exporté. Les extensions ne fournissent pas d'adaptateur de debug : le debug se configure dans `launch.toml`. Les [modules officiels](https://github.com/Ajustor/coding-unicorns-modules) sont les meilleurs exemples à copier.

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

- **Binaire portable** (Linux, macOS, `.exe` Windows) : l'exécutable est remplacé sur place, puis l'IDE redémarre.
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
