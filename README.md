# 🦄 Writing Unicorns

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
- 🔎 Recherche workspace (Ctrl+Shift+F)
- 🔎 Find & Replace dans le fichier (Ctrl+F / Ctrl+H) avec regex

### Git
- 🌿 Branche courante + indicateur ahead/behind
- 📊 Arbre des branches (local/remote) avec graph des commits
- ➕ Stage/unstage par fichier ou en bloc
- 💬 Commit, Push, Pull depuis l'UI
- 🔀 3-panel merge tool (Ours | Result | Theirs) pour les conflits
- 📝 Git blame par ligne
- 🎨 Diff gutter (ajouts/modifications/suppressions)
- 🖱️ Clic droit sur les branches : checkout, merge, delete, create, rename

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
- 🧩 Système de plugins FFI (cdylib) avec manifest TOML
- 📦 Installation depuis git ou dossier local
- 🏗️ Build from sources (workspaces Cargo)
- 🎨 Chaque extension peut fournir : coloration, LSP, hover, DAP

### Configuration
- ⚙️ Thèmes personnalisables (Dark, Monokai, Solarized Dark, One Dark + custom RGB)
- ⌨️ **43 raccourcis clavier configurables** dans Settings → Keybindings
- 💾 Auto-save optionnel

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
git clone https://github.com/votre-utilisateur/writingUnicorns
cd writingUnicorns
cargo build --release
```

Le binaire se trouve dans `target/release/writing-unicorns` (ou `.exe` sur Windows).

```bash
cargo run --release
```

---

## Utilisation

### Premier lancement

1. Lancez l'application
2. **File → Open Folder…** (ou `Ctrl+O`) pour ouvrir un projet
3. Installez des extensions de langage pour obtenir la coloration et le LSP

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
| `Ctrl+Shift+F` | Formater le document |
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

L'éditeur ne contient aucun support de langage intégré. Pour obtenir la coloration syntaxique et le LSP, installez des extensions.

### Extensions officielles

| Extension | Langages | LSP |
|-----------|----------|-----|
| **rust-lang** | `.rs` | rust-analyzer |
| **javascript-lang** | `.js` `.mjs` | typescript-language-server |
| **typescript-lang** | `.ts` | typescript-language-server |
| **react-lang** | `.jsx` `.tsx` | typescript-language-server |
| **python-lang** | `.py` `.pyw` | pylsp |
| **go-lang** | `.go` | gopls |
| **vue-lang** | `.vue` | vue-language-server |
| **svelte-lang** | `.svelte` | svelte-language-server |

### Installer des extensions

#### Depuis un workspace git (plusieurs modules d'un coup)

1. Panel Extensions → **Install Group from Git**
2. Collez l'URL du dépôt (ex: `https://github.com/user/writing-unicorns-modules`)
3. Cliquez **Install All** — tous les modules sont compilés et installés

#### Depuis un dépôt git (un module)

1. Panel Extensions → **Install from Git**
2. Collez l'URL → **Install**

#### Depuis les sources locales

1. Panel Extensions → **Build from Sources**
2. Pointez vers le workspace Cargo contenant les modules
3. **Build & Install All**

### Créer une extension

Chaque extension est un crate Rust compilé en `cdylib` avec un `manifest.toml` :

```toml
[extension]
id = "mon.extension"
name = "Mon Langage"
version = "0.1.0"
description = "Support pour Mon Langage"
author = "Moi"

[capabilities]
languages = ["ml"]
lsp_server = "mon-lsp"
lsp_args = ["--stdio"]

[dependencies]
npm = ["mon-lsp"]
```

Le crate exporte des fonctions FFI C :
- `language_id()` → nom du langage
- `file_extensions()` → extensions supportées (ex: `"ml,mll"`)
- `tokenize_line_ffi(line)` → JSON avec les tokens colorés
- `hover_info_ffi(word, content)` → info de survol
- `free_string(ptr)` → libération mémoire

Template disponible via **Extensions → Create Extension Template**.

---

## Configuration

Le fichier de configuration est stocké dans :

| OS | Chemin |
|----|--------|
| Linux | `~/.config/writing-unicorns/config.toml` |
| macOS | `~/Library/Application Support/writing-unicorns/config.toml` |
| Windows | `%APPDATA%\writing-unicorns\config.toml` |

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
