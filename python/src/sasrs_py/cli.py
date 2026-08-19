"""Télécharge (une fois) et exécute le binaire sasrs précompilé.

Le binaire n'est pas embarqué dans le paquet (il alourdirait le dépôt git
et chaque wheel de ~55 Mo) : il est récupéré depuis une GitHub Release au
premier lancement, vérifié par SHA-256, puis mis en cache localement.
"""

import hashlib
import os
import platform
import subprocess
import sys
import tempfile
import urllib.request
from pathlib import Path

REPO = "LePhilippeDucTai/sasrs"
RELEASE_TAG = "python-v0.1.0"

# (system, machine) -> (nom de l'asset dans la release, sha256 attendu)
_PLATFORM_ASSETS = {
    ("Windows", "AMD64"): (
        "sasrs-windows-x86_64.exe",
        "1b43905e53914873d80150495a2f6ef0b860182cf098426149f99888fb15d466",
    ),
    ("Windows", "x86_64"): (
        "sasrs-windows-x86_64.exe",
        "1b43905e53914873d80150495a2f6ef0b860182cf098426149f99888fb15d466",
    ),
}


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


def _download(asset_name: str, expected_sha256: str, dest: Path) -> None:
    url = f"https://github.com/{REPO}/releases/download/{RELEASE_TAG}/{asset_name}"
    print(f"sasrs : téléchargement de {asset_name} ({url})...", file=sys.stderr)
    dest.parent.mkdir(parents=True, exist_ok=True)
    fd, tmp_name = tempfile.mkstemp(dir=dest.parent)
    tmp_path = Path(tmp_name)
    try:
        with os.fdopen(fd, "wb") as tmp_file:
            with urllib.request.urlopen(url) as response:
                while True:
                    chunk = response.read(1024 * 1024)
                    if not chunk:
                        break
                    tmp_file.write(chunk)
        actual = _sha256(tmp_path)
        if actual != expected_sha256:
            raise SystemExit(
                f"sasrs : empreinte SHA-256 invalide pour {asset_name} "
                f"(attendu {expected_sha256}, obtenu {actual}) — téléchargement corrompu ou altéré."
            )
        tmp_path.chmod(0o755)
        os.replace(tmp_path, dest)
    finally:
        tmp_path.unlink(missing_ok=True)


def _binary_path() -> Path:
    key = (platform.system(), platform.machine())
    asset = _PLATFORM_ASSETS.get(key)
    if asset is None:
        disponibles = ", ".join(sorted(f"{s}/{m}" for s, m in _PLATFORM_ASSETS))
        raise SystemExit(
            f"sasrs : aucun binaire disponible pour {key[0]}/{key[1]}. "
            f"Plateformes disponibles : {disponibles}."
        )
    asset_name, expected_sha256 = asset
    dest = _cache_dir() / asset_name
    if not dest.exists() or _sha256(dest) != expected_sha256:
        _download(asset_name, expected_sha256, dest)
    return dest


def main() -> None:
    binary = _binary_path()
    completed = subprocess.run([str(binary), *sys.argv[1:]])
    raise SystemExit(completed.returncode)


if __name__ == "__main__":
    main()
