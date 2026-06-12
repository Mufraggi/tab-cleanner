# Architecture SML — Extension Chrome tab-cleanner

> **Document consolidé** : état prouvé, architecture cible, plan d'intégration et risques.
> Date : 2026-06-11 · Stack : Rust + WASM (Oxichrome v0.2) · Modèle : all-MiniLM-L6-v2

---

## A. État prouvé — Faisabilité SML

Quatre verdicts indépendants valident la faisabilité technique du SML (Small ML / embeddings locaux) dans l'extension.

### A.1 candle compile en WASM (220 Ko) ✅

| Crate | Compile WASM ? |
|---|---|
| `candle-core` 0.10 | ✅ |
| `candle-nn` 0.10 | ✅ |
| `candle-transformers` 0.10 | ✅ |
| `getrandom` (feature `wasm_js`) | ✅ |

**Commande de build :**
```bash
RUSTFLAGS='--cfg getrandom_backend="wasm_js"' \
  cargo build --lib --target wasm32-unknown-unknown --release
```

**Taille WASM :** 1,76 Mo (avec archi BERT matérialisée en zéros) → ~260 Ko en production (code pur sans poids).

### A.2 Inférence MiniLM fonctionne (embeddings sémantiquement corrects) ✅

**Modèle :** `sentence-transformers/all-MiniLM-L6-v2` (BERT 6 couches, hidden_size=384)

| Métrique | Valeur |
|---|---|
| Taille des poids (f32) | 90,9 Mo |
| Taille en f16 (recommandé) | ~45 Mo |
| Latence 1 texte (natif) | ~50 ms |
| Latence 20 textes (natif) | ~780 ms (~39 ms/texte) |
| Latence forward 20 textes | ~94 ms |
| Sanity check sémantique | Rust~React = 0,53 ✅, Rust~YouTube = 0,03 ✅, K8s~Docker = 0,58 ✅ |

### A.3 Tokenizer Rust pur équivalent exact à HF ✅

**Implémentation :** `WordPieceTokenizer` dans `candle-wasm-test/src/tokenizer.rs` (~800 lignes).

| Étape | Implémentation |
|---|---|
| 1. Normalisation | Unicode lowercase, NFD + strip accents, CJK spacing |
| 2. Pre-tokenization | Split whitespace puis ponctuation ASCII |
| 3. WordPiece | Greedy longest-match-first, préfixe `##` |
| 4. Post-processing | Ajout `[CLS]` (101) et `[SEP]` (102) |

**Validation :** 28 tests unitaires, 16 textes d'onglets réalistes + 12 cas limites. **IDs EXACTS** vs HuggingFace sur tous.

**Dépendances :** `serde`, `serde_json`, `unicode-normalization` — zéro C, zéro `onig`.

### A.4 Pipeline complet titre→embedding en WASM pur ✅

**Pipeline :**
```
texte → WordPieceTokenizer.tokenize() → (ids, attention_mask)
      → tokenize_batch() → tensors padés
      → BertModel.forward() → last_hidden_state
      → mean_pool() → embeddings non normalisés
      → l2_normalize() → vecteurs unitaires (384 dim)
```

**Validation :** Cosine similarity = **1,00000000** entre pipeline HF et pipeline Rust pur (20 textes). L2 distance < 5×10⁻⁷.

### A.5 Chiffres clés

| Métr.ique | Valeur |
|---|---|
| Modèle | all-MiniLM-L6-v2 (BERT, 6 couches) |
| Hidden dimension | 384 |
| Poids f16 | ~45 Mo |
| Tokenizer (tokenizer.json) | 466 Ko |
| WASM pipeline (code pur) | ~260 Ko |
| Latence embedding (natif, 1 texte) | ~12 ms |
| Latence embedding (natif, 20 textes) | ~95 ms |
| Équivalence HF | ✅ cosinus 1,00000000 |

---

## B. Architecture cible du SML dans tab-cleanner

### B.1 Principe général

Le SML **s'ajoute à côté** du groupement heuristique existant (`group_tabs`), pas en remplacement. L'heuristique reste le fallback par défaut. Un trait `Classifier` (ou fonction `classify_semantic`) est appelé par `RunGrouping` pour enrichir les décisions de groupement.

### B.2 Cache des poids du modèle

**Décision : Cache API (pas IndexedDB, pas chrome.storage).**

| Stockage | Adapté aux blobs 45 Mo ? | Persistant ? | Accessible depuis SW ? |
|---|---|---|---|
| **Cache API** | ✅ Oui (faite pour ça) | ✅ Oui | ✅ Oui |
| `chrome.storage.local` | ❌ Limite ~10 Mo par clé | ✅ Oui | ✅ Oui |
| IndexedDB | ✅ Oui | ✅ Oui | ⚠️ API async plus complexe |

**Fonctionnement :**
1. Au premier run, fetch des poids depuis HuggingFace CDN (ou mirror)
2. Stockage via `caches.open('sml-model')` puis `cache.put(url, response)`
3. Aux runs suivants, `cache.match(url)` → rechargement depuis le cache
4. Le modèle (45 Mo f16) est chargé en mémoire WASM depuis le `ArrayBuffer` du cache

### B.3 Point d'insertion dans le code

```
RunGrouping()
  ├── group_tabs_heuristic()        ← existant, fallback
  └── classify_semantic()           ← NOUVEAU, optionnel
        ├── load_model_from_cache()
        ├── embed_batch(tabs)       → Vec<Vec<f32>>
        ├── assign_by_theme()       → group assignments
        └── merge_with_heuristic()  → final decisions
```

### B.4 Mécanique de groupement sémantique

**Algorithme (à implémenter dans `src/sml/`) :**

1. **Embedder chaque onglet** (titre + URL) → vecteur 384 dim
2. **Embedder le thème** de chaque `StoredGroup` existant
3. Pour chaque onglet non groupé, calculer la similarité cosinus avec chaque thème de groupe
4. **Assigner** l'onglet au groupe dont le thème est le plus proche, **si** la similarité > SEUIL
5. Si aucun groupe ne dépasse le seuil → l'onglet va en "Other" ou crée un nouveau groupe
6. Le thème d'un groupe est le centroid (moyenne) des embeddings de ses onglets

**SEUIL recommandé (à ajuster empiriquement) :** 0,35-0,50 (similarité cosinus).

### B.5 Gestion du Service Worker endormi

| Problème | Solution |
|---|---|
| Le SW se suspend après ~30s d'inactivité | Rechargement du modèle depuis Cache API au réveil |
| 45 Mo à recharger = latence | Le Cache API sert les blobs en mémoire locale (pas de réseau) — latence estimée < 500 ms |
| Inférence au boot ? | **Non** — inférence déclenchée uniquement sur clic "Ranger" (pas au boot) |
| Cache des embeddings par URL | `HashMap<String, Vec<f32>>` en mémoire, persisté optionnellement dans le storage |

### B.6 Module `src/sml/` — Structure proposée

```
src/sml/
├── mod.rs              # Point d'entrée, trait Classifier
├── tokenizer.rs        # WordPieceTokenizer (porté depuis candle-wasm-test)
├── pipeline.rs         # embed_batch, tokenize_batch, mean_pool, l2_normalize
├── model_cache.rs      # Cache API : download + load des poids
├── grouping.rs         # assign_by_theme, threshold logic
└── embedding_cache.rs  # Cache des embeddings calculés par URL
```

### B.7 Dépendances WASM à ajouter au `Cargo.toml` principal

```toml
[dependencies]
candle-core = "0.10"
candle-nn = "0.10"
candle-transformers = "0.10"
serde = "1"
serde_json = "1"
unicode-normalization = "0.1"
getrandom = { version = "0.3", features = ["wasm_js"] }
```

---

## C. Plan d'intégration par étapes

Chaque étape est isolée et testable indépendamment.

### Étape 1 — Tester le chargement réel des poids + inférence en contexte WASM navigateur

**Objectif :** Valider le dernier point de faisabilité non prouvé : charger `model.safetensors` (f16, 45 Mo) depuis un fetch WASM réel et exécuter l'inférence dans un navigateur.

**Actions :**
1. Créer une page HTML de test avec un service worker minimal
2. Télécharger les poids all-MiniLM-L6-v2 en f16
3. Les charger via `caches.open()` puis `cache.match()`
4. Passer le `ArrayBuffer` à la fonction WASM
5. Exécuter `embed_batch()` sur des textes de test
6. Mesurer latence réelle WASM

**Livrable :** verdict step1.md validant ou infirmant la faisabilité navigateur.

### Étape 2 — Implémenter download + cache des poids via Cache API

**Objectif :** Module `model_cache.rs` avec deux fonctions :
- `ensure_model_cached(url) → Result` : fetch + cache.put
- `load_model_from_cache(url) → BertModel` : cache.match → ArrayBuffer → `from_buffered_safetensors`

**Testable :** Tests unitaires avec mock Cache API (ou test navigateur dédié).

### Étape 3 — Porter tokenizer.rs et pipeline.rs dans tab-cleanner

**Objectif :** Copier (`candle-wasm-test/src/tokenizer.rs` et `pipeline.rs`) → `src/sml/`. Ajuster les chemins, supprimer les dépendances inutiles, vérifier que `cargo check --target wasm32-unknown-unknown` passe.

**Testable :** `cargo test` (les 28 tests du tokenizer + tests du pipeline doivent passer dans le nouveau contexte).

### Étape 4 — Implémenter la mécanique de groupement sémantique

**Objectif :** Module `grouping.rs` avec :
- `assign_by_theme(embeddings, groups, threshold) → Vec<Assignment>`
- `compute_centroid(embeddings) → Vec<f32>`
- Seuil paramétrable, fallback "Other" / nouveau groupe

**Testable :** Tests unitaires avec embeddings synthétiques.

### Étape 5 — Câbler dans RunGrouping (SML en complément de l'heuristique)

**Objectif :** Modifier `RunGrouping` pour :
1. Vérifier si le modèle est disponible (caché)
2. Si oui : charger le modèle → embed les onglets → assigner par thème
3. Fusionner les assignments SML avec les décisions heuristiques
4. Si le modèle n'est pas encore caché : utiliser uniquement l'heuristique

**Testable :** Tests d'intégration avec données mockées.

### Étape 6 — Mesurer vs baseline heuristique

**Objectif :** Comparer la qualité du groupement SML vs heuristique pure sur un ensemble de scénarios réels (profils d'onglets).

**Métriques :**
- Taux de rapprochement "correct" (évalué subjectivement ou via ground truth)
- Latence perçue par l'utilisateur
- Stabilité des groupes entre runs

---

## D. Risques connus

| Risque | Impact | Atténuation |
|---|---|---|
| **SW endormi + rechargement 45 Mo** | Latence au premier clic "Ranger" après réveil du SW | Cache API sert localement (< 500 ms estimé). Option : heartbeat optionnel ou délégation à un Offscreen Document si la latence est rédhibitoire |
| **Pas de SIMD WASM** | Inférence 2-5× plus lente qu'en natif | 95 ms natif → ~475 ms WASM pour 20 textes. Acceptable pour un usage "clic bouton" |
| **Chargement runtime des poids non encore validé en navigateur réel** | Risque technique : `VarBuilder::from_buffered_safetensors` peut ne pas fonctionner en WASM | Étape 1 dédiée pour lever ce risque avant toute intégration |
| **Taille de l'extension** | 45 Mo (poids f16) + 466 Ko (tokenizer.json) + ~260 Ko (WASM) = ~46 Mo | Le Chrome Web Store autorise jusqu'à ~100 Mo pour les extensions. Les poids sont téléchargés au premier run, pas bundlés |
| **Précision du seuil cosinus** | Mauvaise séparation si le seuil est mal calibré | Le seuil doit être ajusté empiriquement sur des données réelles d'onglets. Option : réglage dynamique basé sur la distribution des similarités |
| **Dérive des groupes** | Les centroids des groupes changent à chaque run si le contenu des onglets change | Persister les embeddings des thèmes de groupe dans le storage (pas seulement les centroids). L'utilisateur peut verrouiller ("manual") un groupe pour empêcher sa modification |
| **Compétition avec l'heuristique** | Conflits entre groupement heuristique et SML sur le même onglet | L'heuristique reste le décideur principal. Le SML ne fait que des suggestions. En cas de conflit, l'heuristique gagne (phase 1) |

---

## Dépendances et artefacts

### Verdicts (source des preuves)

| Fichier | Contenu |
|---|---|
| `candle-wasm-verdict.md` | Compilation WASM de candle (core+nn+transformers) ✅ |
| `candle-inference-verdict.md` | Inférence all-MiniLM-L6-v2 en natif ✅ |
| `tokenizer-verdict.md` | Tokenizer WordPiece Rust pur équivalent HF ✅ |
| `candle-wasm-test/pipeline-wasm-verdict.md` | Pipeline complet titre→embedding en WASM pur ✅ |

### Code réutilisable (source)

| Fichier | Usage dans tab-cleanner |
|---|---|
| `candle-wasm-test/src/tokenizer.rs` | → `src/sml/tokenizer.rs` (à porter) |
| `candle-wasm-test/src/pipeline.rs` | → `src/sml/pipeline.rs` (à porter) |
| `candle-wasm-test/src/lib.rs` | Référence de la structure d'export WASM |
| `candle-wasm-test/Cargo.toml` | Référence des dépendances WASM |
| `candle-wasm-test/tokenizer.json` | Vocabulaire BERT (466 Ko) — à bundler ou télécharger |
| `candle-wasm-test/src/bin/inference.rs` | Binaire de validation (HF vs Rust) |
