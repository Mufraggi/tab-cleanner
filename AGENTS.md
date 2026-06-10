# Projet : extension Chrome de rangement d'onglets
Stack : Rust + WASM via Oxichrome v0.2 (https://oxichrome.dev/docs).
Tabs/Storage wrappés par Oxichrome. tabGroups NON wrappé → bindings FFI wasm-bindgen custom requis.
Pas de ML en phase 1 : groupement heuristique (domaine, mots-clés).
Persister l'état des groupes dans storage pour réutilisation entre runs.