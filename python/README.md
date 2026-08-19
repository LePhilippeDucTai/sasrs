# sasrs (wrapper Python)

Ce package n'est pas une réimplémentation de `sasrs` en Python : c'est un
wrapper léger (aucune dépendance hors bibliothèque standard) qui, au
premier lancement, télécharge le binaire Rust précompilé (`sasrs.exe`,
Windows x86_64) depuis une [GitHub Release](https://github.com/LePhilippeDucTai/sasrs/releases)
du dépôt, vérifie son empreinte SHA-256, le met en cache localement, puis
l'exécute via `subprocess`. Il permet d'utiliser `sasrs` sur une machine
où Rust n'est pas installé, du moment que Python et un accès réseau vers
`github.com` le sont.

Voir le [README principal](../README.md) pour la documentation de
l'interpréteur lui-même.
