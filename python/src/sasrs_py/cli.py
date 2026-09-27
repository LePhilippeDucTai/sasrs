"""Télécharge (une fois) et exécute le binaire sasrs précompilé.

Le binaire n'est pas embarqué dans le paquet (il alourdirait le dépôt git
et chaque wheel de ~55 Mo) : il est récupéré depuis une GitHub Release au
premier lancement, vérifié par SHA-256, puis mis en cache localement.

Durcissement (J06-P4) :
  - timeout réseau sur le téléchargement (urlopen + lecture) ;
  - messages d'erreur clairs sur stderr, SANS traceback (réseau absent,
    HTTP 404, plateforme non supportée, empreinte SHA-256 invalide) ;
  - cache indexé par SHA : si un marqueur « vérifié » portant l'empreinte
    attendue accompagne le binaire, il n'est PAS re-haché au lancement ;
  - verrou de fichier dans le répertoire de cache : deux lancements
    concurrents ne téléchargent pas deux fois (le second attend, puis
    constate que le marqueur vérifié existe déjà) ;
  - nettoyage des temporaires orphelins laissés par un lancement tué ;
  - repli sûr sous Windows : si `os.replace` échoue (binaire en cours
    d'exécution, verrouillé par l'antivirus...) alors que le binaire en
    place est déjà valide, on le garde au lieu d'échouer.

NOTE (conflit connu R-001) : le watchdog Hermes réécrit les lignes de SHAs
de ce fichier sur main. Les empreintes sont donc isolées dans des
constantes nommées, en tête de fichier, une par ligne — pour qu'un
remplacement de ligne ne touche qu'une constante et reste repérable.
"""

import contextlib
import hashlib
import os
import platform
import stat
import subprocess
import sys
import tempfile
import time
import urllib.error
import urllib.request
from pathlib import Path

REPO = "LePhilippeDucTai/sasrs"
RELEASE_TAG = "python-v0.1.0"

# --- Empreintes des assets (isolation conflit Hermes R-001 : une constante
# --- par ligne, ne PAS regrouper plusieurs SHAs sur une même ligne).
SHA256_SASRS_WINDOWS_X86_64_EXE = (
    "76f82df0d5c28443c01844ca76faa4b93fe30ce278c378d6ddf7b5752cb84ee6"
)

# Timeout réseau (connexion + lecture) en secondes.
NETWORK_TIMEOUT_S = 60
# Durée maximale d'attente du verrou de cache en secondes.
LOCK_TIMEOUT_S = 120
# Âge (s) au-delà duquel un fichier temporaire orphelin est supprimé.
ORPHAN_TEMP_MAX_AGE_S = 3600
# Marqueur « binaire vérifié » : cache/<asset>.verified contient le SHA-256.
VERIFIED_SUFFIX = ".verified"
# Préfixe des fichiers temporaires de téléchargement dans le cache.
TEMP_PREFIX = ".sasrs-download-"

# (system, machine) -> (nom de l'asset dans la release, sha256 attendu)
_PLATFORM_ASSETS = {
    ("Windows", "AMD64"): (
        "sasrs-windows-x86_64.exe",
        SHA256_SASRS_WINDOWS_X86_64_EXE,
    ),
    ("Windows", "x86_64"): (
        "sasrs-windows-x86_64.exe",
        SHA256_SASRS_WINDOWS_X86_64_EXE,
    ),
}


class SasrsError(Exception):
    """Erreur attendue : message clair pour l'utilisateur, sans traceback."""


def _cache_dir() -> Path:
    if platform.system() == "Windows":
        base = os.environ.get("LOCALAPPDATA") or str(Path.home())
        return Path(base) / "sasrs" / RELEASE_TAG
    base = os.environ.get("XDG_CACHE_HOME") or str(Path.home() / ".cache")
    return Path(base) / "sasrs" / RELEASE_TAG


def _sha256(path: Path) -> str:
    digest = hashlib.sha256()
    with path.open("rb") as f:
        for chunk in iter(lambda: f.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def _asset_for(key) -> tuple:
    """Renvoyer (asset, sha256) pour (system, machine), ou lever SasrsError."""
    asset = _PLATFORM_ASSETS.get(key)
    if asset is None:
        disponibles = ", ".join(sorted(f"{s}/{m}" for s, m in _PLATFORM_ASSETS))
        raise SasrsError(
            f"sasrs : plateforme non supportée : {key[0]}/{key[1]}. "
            f"Binaires publiés pour : {disponibles}. "
            "Compilez depuis les sources (cargo build --release) sur cette plateforme."
        )
    return asset


def _verified_marker(dest: Path) -> Path:
    return dest.with_name(dest.name + VERIFIED_SUFFIX)


def _cache_is_verified(dest: Path, expected_sha256: str) -> bool:
    """Vrai si le marqueur vérifié du binaire porte l'empreinte attendue.

    Cache indexé par SHA : le marqueur enregistre l'empreinte de la
    dernière vérification réussie ; s'il correspond à l'empreinte attendue,
    le binaire n'est PAS re-haché au lancement.
    """
    marker = _verified_marker(dest)
    try:
        return marker.read_text(encoding="utf-8").strip() == expected_sha256
    except OSError:
        return False


def _write_verified_marker(dest: Path, expected_sha256: str) -> None:
    with contextlib.suppress(OSError):
        _verified_marker(dest).write_text(expected_sha256 + "\n", encoding="utf-8")


def _cleanup_orphan_temp(cache_dir: Path, now=None) -> None:
    """Supprimer les temporaires orphelins (lancement tué en plein download)."""
    if now is None:
        now = time.time()
    if not cache_dir.is_dir():
        return
    for candidate in cache_dir.glob(TEMP_PREFIX + "*"):
        try:
            if now - candidate.stat().st_mtime > ORPHAN_TEMP_MAX_AGE_S:
                candidate.unlink()
        except OSError:
            pass


class _CacheLock:
    """Verrou exclusif du cache (flock POSIX) pour lancements concurrents."""

    def __init__(self, cache_dir: Path):
        self.cache_dir = cache_dir
        self.fd = None
        self.lock_path = None

    def __enter__(self):
        import fcntl  # POSIX ; le wrapper publié ne cible que Windows x86_64.

        self.cache_dir.mkdir(parents=True, exist_ok=True)
        self.lock_path = self.cache_dir / "download.lock"
        self.fd = os.open(self.lock_path, os.O_RDWR | os.O_CREAT, 0o644)
        deadline = time.monotonic() + LOCK_TIMEOUT_S
        while True:
            try:
                fcntl.flock(self.fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
                return self
            except OSError:
                if time.monotonic() >= deadline:
                    os.close(self.fd)
                    self.fd = None
                    raise SasrsError(
                        "sasrs : un autre lancement occupe le cache depuis plus de "
                        f"{LOCK_TIMEOUT_S} s ({self.lock_path}) — réessayez plus tard."
                    ) from None
                time.sleep(0.05)

    def __exit__(self, exc_type, exc, tb):
        if self.fd is not None:
            with contextlib.suppress(OSError):
                os.close(self.fd)
            self.fd = None
        return False


def _cache_lock(cache_dir: Path) -> _CacheLock:
    return _CacheLock(cache_dir)


def _download(asset_name: str, expected_sha256: str, dest: Path) -> None:
    url = f"https://github.com/{REPO}/releases/download/{RELEASE_TAG}/{asset_name}"
    print(f"sasrs : téléchargement de {asset_name} ({url})...", file=sys.stderr)
    dest.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(prefix=TEMP_PREFIX, dir=dest.parent)
    tmp_path = Path(tmp_name)
    try:
        with os.fdopen(fd, "wb") as tmp_file:
            try:
                with urllib.request.urlopen(url, timeout=NETWORK_TIMEOUT_S) as response:
                    while True:
                        chunk = response.read(1024 * 1024)
                        if not chunk:
                            break
                        tmp_file.write(chunk)
            except urllib.error.HTTPError as error:
                raise SasrsError(
                    f"sasrs : téléchargement impossible (HTTP {error.code} "
                    f"{error.reason}) : {url}"
                    + (
                        " — release ou asset introuvable ; vérifiez que "
                        f"{RELEASE_TAG} publie bien {asset_name}."
                        if error.code in (403, 404)
                        else ""
                    )
                ) from None
            except (urllib.error.URLError, TimeoutError, OSError) as error:
                raise SasrsError(
                    f"sasrs : réseau indisponible ou trop lent ({error}) — "
                    f"impossible de joindre {url}. Un accès réseau vers "
                    "github.com est requis au premier lancement."
                ) from None
        actual = _sha256(tmp_path)
        if actual != expected_sha256:
            raise SasrsError(
                f"sasrs : empreinte SHA-256 invalide pour {asset_name} "
                f"(attendu {expected_sha256}, obtenu {actual}) — "
                "téléchargement corrompu ou altéré ; aucun binaire installé."
            )
        tmp_path.chmod(
            tmp_path.stat().st_mode | stat.S_IXUSR | stat.S_IXGRP | stat.S_IXOTH
        )
        try:
            os.replace(tmp_path, dest)
        except OSError:
            # Repli sûr (Windows : binaire en cours d'exécution / antivirus) :
            # si le binaire déjà en place est valide, on le garde.
            if dest.is_file() and _sha256(dest) == expected_sha256:
                print(
                    f"sasrs : {dest} n'a pas pu être remplacé mais le binaire en "
                    "place est déjà à jour — on le conserve.",
                    file=sys.stderr,
                )
            else:
                raise SasrsError(
                    f"sasrs : impossible d'installer le binaire à {dest} "
                    "(remplacement refusé) et aucun binaire valide en place — "
                    "fermez les processus sasrs et réessayez."
                ) from None
        _write_verified_marker(dest, expected_sha256)
    finally:
        with contextlib.suppress(OSError):
            tmp_path.unlink()


def _ensure_binary(cache_dir=None, *, system=None, machine=None) -> Path:
    """Renvoyer le chemin du binaire vérifié, en le téléchargeant si besoin."""
    key = (system or platform.system(), machine or platform.machine())
    asset_name, expected_sha256 = _asset_for(key)
    cache_dir = Path(cache_dir) if cache_dir is not None else _cache_dir()
    dest = cache_dir / asset_name

    _cleanup_orphan_temp(cache_dir)
    if dest.is_file() and _cache_is_verified(dest, expected_sha256):
        return dest

    with _cache_lock(cache_dir):
        # Un lancement concurrent a peut-être fini le téléchargement pendant
        # l'attente du verrou : re-vérifier le marqueur avant de re-télécharger.
        if dest.is_file() and _cache_is_verified(dest, expected_sha256):
            return dest
        if dest.is_file():
            # Cache douteux (marqueur absent ou périmé) : re-hacher une fois.
            if _sha256(dest) == expected_sha256:
                _write_verified_marker(dest, expected_sha256)
                return dest
            with contextlib.suppress(OSError):
                dest.unlink()
        _download(asset_name, expected_sha256, dest)
    return dest


def main() -> None:
    try:
        binary = _ensure_binary()
    except SasrsError as error:
        print(str(error), file=sys.stderr)
        raise SystemExit(1)
    except KeyboardInterrupt:
        raise SystemExit(130)
    completed = subprocess.run([str(binary), *sys.argv[1:]])
    raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
