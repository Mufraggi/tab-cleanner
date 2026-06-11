# Phase B3-B — Éditions robustes au worker endormi (popup persiste direct, background applique à Chrome)

## Status ✅ Complete

### Modified files
| # | Task | File | Status |
|---|------|------|--------|
| 1 | Add `persist_group_fields()` function (popup lit/écrit storage direct) | `src/popup.rs` | ✅ |
| 2 | Add `send_update_group_best_effort()` wrapper (log silencieux) | `src/popup.rs` | ✅ |
| 3 | Rewrite `commit_rename` — persist direct → refresh → best-effort | `src/lib.rs` | ✅ |
| 4 | Rewrite `on_color_change` — local update → persist direct → refresh → best-effort | `src/lib.rs` | ✅ |
| 5 | Rewrite `on_theme_change` — persist direct → refresh ONLY (pas d'appel background) | `src/lib.rs` | ✅ |

### Verification
- `cargo check` — ✅ 1 pre-existing dead_code warning (unchanged)
- `cargo test` — ✅ 68/68 pass (no regression)

## Changes detail

### 1. `src/popup.rs` — Nouvelles fonctions

**`persist_group_fields(name, display_name, color, theme)`** (l. 182)
- Lit `GroupState` depuis `storage::get(GROUP_STATE_KEY)`
- Trouve le `StoredGroup` par `name`
- Applique les champs fournis (`display_name`, `color`, `theme`) avec `Option<&str>`
- Réécrit le storage via `storage::set()`
- Retourne `Result<(), String>` — pas de `unwrap`, pas de perte de données

**`send_update_group_best_effort(name, display_name, color, theme)`** (l. 226)
- Appelle `send_update_group()` et ignore l'erreur silencieusement avec log
- Utilise `oxichrome::log!` pour tracer, jamais d'erreur rouge

### 2. `src/lib.rs` — Trois handlers rewrités

**`commit_rename`** (l. 100)
1. Optimistic UI update (signal local — déjà présent)
2. `persist_group_fields()` → enregistre `display_name` dans le storage **directement**
3. `fetch_popup_data()` → rafraîchit l'UI depuis le storage
4. `send_update_group_best_effort()` → notifie le worker (best-effort)

**`on_color_change`** (l. 163)
1. Met à jour l'affichage immédiatement (change `color_name` et `color_hex` dans le signal local)
2. `persist_group_fields()` → enregistre `color` dans le storage **directement**
3. `fetch_popup_data()` → rafraîchit l'UI depuis le storage
4. `send_update_group_best_effort()` → notifie le worker pour appliquer la couleur Chrome (best-effort)

**`on_theme_change`** (l. 213)
1. `persist_group_fields()` → enregistre `theme` dans le storage **directement** (seulement)
2. `fetch_popup_data()` → rafraîchit l'UI depuis le storage
3. **Aucun appel background** — le thème n'a pas d'effet Chrome natif

### Contraintes respectées
- ✅ Aucun `unwrap` runtime, ni dans popup ni dans messaging
- ✅ Les éditions n'affichent jamais d'erreur rouge à cause du worker endormi
- ✅ `popup` reste dans `lib.rs`, versions de deps inchangées
- ✅ Signatures FFI tab_groups réelles lues et conservées
- ✅ `cargo check` + `cargo test` passent
- ✅ Build via `cargo oxichrome build`, jamais `wasm-pack`
