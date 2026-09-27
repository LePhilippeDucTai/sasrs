# sasrs (wrapper Python)

Ce package n'est pas une réimplémentation de `sasrs` en Python : c'est un
wrapper léger (aucune dépendance hors bibliothèque standard) qui, au
premier lancement, télécharge le binaire Rust précompilé (`sasrs.exe`,
Windows x86_64) depuis une [GitHub Release](https://github.com/LePhilippeDucTai/sasrs/releases)
du dépôt, vérifie son empreinte SHA-256, le met en cache localement, puis
l'exécute via `subprocess`. Il permet d'utiliser `sasrs` sur une machine
où Rust n'est pas installé, du moment que Python et un accès réseau vers
`github.com` le sont.

## Comportement

- **Téléchargement unique** : le binaire est récupéré depuis la release
  `python-v0.1.0`, avec un timeout réseau ; toute erreur (réseau absent,
  HTTP 404, empreinte invalide, plateforme non supportée) produit un
  message clair sur stderr et un code retour 1 — jamais de traceback.
- **Cache indexé par SHA** : après vérification de l'empreinte, un
  marqueur `<binaire>.verified` contenant le SHA-256 attendu est écrit ;
  aux lancements suivants, si le marqueur correspond, le binaire n'est
  **pas** re-haché. Un cache corrompu (marqueur absent ou périmé) est
  re-haché puis re-téléchargé si l'empreinte ne correspond pas.
- **Lancements concurrents** : un verrou de fichier dans le répertoire de
  cache sérialise les téléchargements — deux `sasrs` lancés en parallèle
  ne téléchargent qu'une fois.
- **Nettoyage** : les fichiers temporaires orphelins (lancement tué en
  plein téléchargement) sont supprimés au lancement suivant.
- **Repli Windows** : si le remplacement du binaire échoue (`os.replace`
  refusé — binaire en cours d'exécution, antivirus) alors qu'un binaire
  valide est déjà en place, celui-ci est conservé.

## Limites

- Seul **Windows x86_64** a un binaire publié ; sur toute autre plateforme
  le wrapper refuse de s'exécuter (compilez depuis les sources).
- Un accès réseau vers `github.com` est requis **au premier lancement**
  (et après chaque changement d'empreinte attendue, ex. nouvelle release) ;
  hors ligne, seul un cache déjà vérifié fonctionne.
- Si le marqueur `.verified` est présent, le binaire en cache est estimé
  fiable sans re-hachage : modifier manuellement le binaire **et** son
  marqueur contournerait la vérification (le marqueur est protégé par
  l'empreinte attendue, pas par une signature).
- Le SHA-256 des assets est une constante du wrapper (`cli.py`) : il doit
  être mis à jour à chaque nouvelle release publiée.

Les tests (bibliothèque standard, réseau et plateforme simulés) :
`PYTHONPATH=python/src python3 -m unittest discover -s python/tests -v`.

Voir le [README principal](../README.md) pour la documentation de
l'interpréteur lui-même.
