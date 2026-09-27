"""Tests du wrapper Python sasrs (J06-P4) — bibliothèque standard uniquement.

`urlopen` et la plateforme sont SIMULÉS (unittest.mock) : aucun test ne
parle au réseau ni ne dépend de la machine hôte.

Cas couverts : cache absent, cache corrompu, réseau absent, plateforme non
supportée, deux lancements concurrents, SHA incorrect — plus le nettoyage
des temporaires orphelins et le repli Windows sur échec d'`os.replace`.
"""

import contextlib
import hashlib
import os
import shutil
import threading
import time
import unittest
import urllib.error
import unittest.mock as mock
from pathlib import Path

from sasrs_py import cli

ASSET = "sasrs-windows-x86_64.exe"
GOOD_SHA = cli.SHA256_SASRS_WINDOWS_X86_64_EXE
KEY_WINDOWS = ("Windows", "AMD64")


class FakeResponse:
    """Objet renvoyé par urlopen simulé : context manager + read()."""

    def __init__(self, payload):
        self._payload = payload

    def read(self, size=-1):
        if not self._payload:
            return b""
        chunk, self._payload = self._payload[:size], self._payload[size:]
        return chunk

    def __enter__(self):
        return self

    def __exit__(self, exc_type, exc, tb):
        return False


def patch_urlopen(payload):
    """Simuler urlopen : renvoie un context manager patché qui enregistre les appels."""
    calls = []

    def fake_urlopen(url, timeout=None):
        calls.append((url, timeout))
        return FakeResponse(payload)

    return mock.patch.object(cli.urllib.request, "urlopen", side_effect=fake_urlopen), calls


class WrapperTestCase(unittest.TestCase):
    def setUp(self):
        import tempfile

        cache = Path(tempfile.mkdtemp(prefix="sasrs-py-tests-"))
        self.addCleanup(lambda: shutil.rmtree(cache, ignore_errors=True))
        self.cache = cache
        # L'asset « publié » attendu est simulé : SHA de b"BINAIRE".
        self.expected_sha = hashlib.sha256(b"BINAIRE").hexdigest()
        patcher = mock.patch.dict(
            cli._PLATFORM_ASSETS,
            {KEY_WINDOWS: (ASSET, self.expected_sha)},
            clear=True,
        )
        patcher.start()
        self.addCleanup(patcher.stop)
        # Plateforme fixée : Windows/AMD64 quel que soit l'hôte de test.
        p_system = mock.patch.object(cli.platform, "system", return_value=KEY_WINDOWS[0])
        p_machine = mock.patch.object(cli.platform, "machine", return_value=KEY_WINDOWS[1])
        p_system.start()
        p_machine.start()
        self.addCleanup(p_system.stop)
        self.addCleanup(p_machine.stop)


    def binary(self):
        return self.cache / ASSET


@contextlib.contextmanager
def _tempdir():
    import tempfile

    with tempfile.TemporaryDirectory(prefix="sasrs-py-tests-") as directory:
        yield Path(directory)


class TestCacheAbsent(WrapperTestCase):
    def test_premier_lancement_telecharge_puis_marque_vérifié(self):
        patcher, calls = patch_urlopen(b"BINAIRE")
        with patcher:
            dest = cli._ensure_binary(cache_dir=self.cache)
        self.assertEqual(dest, self.binary())
        self.assertTrue(dest.is_file())
        self.assertEqual(calls[0][1], cli.NETWORK_TIMEOUT_S)
        # Deuxième lancement : cache indexé par SHA, aucun re-téléchargement
        # (et donc aucun re-hachage — urlopen ne serait pas appelé non plus).
        with patch_urlopen(b"X")[0]:
            dest2 = cli._ensure_binary(cache_dir=self.cache)
        self.assertEqual(dest2, dest)

    def test_marqueur_vérifié_évite_le_re_hachage(self):
        with patch_urlopen(b"BINAIRE")[0]:
            cli._ensure_binary(cache_dir=self.cache)
        # Même un binaire au contenu invalide n'est pas re-haché tant que le
        # marqueur « vérifié » porte l'empreinte attendue (cache indexé par SHA).
        self.binary().write_bytes(b"CORROMPU-MAIS-CONFIANT")
        self.assertTrue(cli._cache_is_verified(self.binary(), self.expected_sha))
        patcher, calls = patch_urlopen(b"AUTRE")
        with patcher:
            cli._ensure_binary(cache_dir=self.cache)
        self.assertEqual(calls, [])


class TestCacheCorrompu(WrapperTestCase):
    def test_cache_corrompu_sans_marqueur_est_re_téléchargé(self):
        self.cache.mkdir(parents=True, exist_ok=True)
        self.binary().write_bytes(b"CONTENU CORROMPU")
        self.assertFalse(self.binary().with_name(ASSET + cli.VERIFIED_SUFFIX).exists())
        with patch_urlopen(b"BINAIRE")[0]:
            dest = cli._ensure_binary(cache_dir=self.cache)
        self.assertEqual(dest.read_bytes(), b"BINAIRE")

    def test_marqueur_périmé_force_le_re_hachage(self):
        self.cache.mkdir(parents=True, exist_ok=True)
        self.binary().write_bytes(b"CONTENU CORROMPU")
        cli._write_verified_marker(self.binary(), "0" * 64)  # autre SHA
        with patch_urlopen(b"BINAIRE")[0]:
            dest = cli._ensure_binary(cache_dir=self.cache)
        self.assertEqual(dest.read_bytes(), b"BINAIRE")
        self.assertTrue(cli._cache_is_verified(dest, self.expected_sha))


class TestReseauAbsent(WrapperTestCase):
    def test_urerror_message_clair_sans_binaire(self):
        error = urllib.error.URLError(OSError("Network is unreachable"))

        def fake_urlopen(url, timeout=None):
            raise error

        with mock.patch.object(cli.urllib.request, "urlopen", side_effect=fake_urlopen):
            with self.assertRaises(cli.SasrsError) as ctx:
                cli._ensure_binary(cache_dir=self.cache)
        self.assertIn("réseau", str(ctx.exception))
        self.assertFalse(self.binary().exists())
        # Aucun temporaire orphelin laissé par l'échec.
        self.assertEqual(list(self.cache.glob(cli.TEMP_PREFIX + "*")), [])

    def test_main_sort_1_sans_traceback(self):
        def fake_urlopen(url, timeout=None):
            raise urllib.error.URLError("name resolution failed")

        with mock.patch.object(cli.urllib.request, "urlopen", side_effect=fake_urlopen):
            with mock.patch.object(cli.sys, "argv", ["sasrs", "--version"]):
                with self.assertRaises(SystemExit) as ctx:
                    cli.main()
        self.assertEqual(ctx.exception.code, 1)


class TestPlateformeNonSupportee(WrapperTestCase):
    def test_plateforme_inconnue_message_explicitant_les_binaires(self):
        with self.assertRaises(cli.SasrsError) as ctx:
            cli._ensure_binary(cache_dir=self.cache, system="Plan9", machine="m68k")
        message = str(ctx.exception)
        self.assertIn("Plan9/m68k", message)
        self.assertIn("Windows/AMD64", message)  # plateformes disponibles listées

    def test_main_sort_1(self):
        with mock.patch.object(cli.platform, "system", return_value="SunOS"):
            with self.assertRaises(SystemExit) as ctx:
                cli.main()
        self.assertEqual(ctx.exception.code, 1)


class TestShaIncorrect(WrapperTestCase):
    def test_sha_invalide_rejette_le_téléchargement(self):
        # L'empreinte attendue (constante du wrapper) ne correspond pas au
        # payload téléchargé : fichier corrompu ou altéré -> rejet.
        payload = b"PAYLOAD MALVEILLANT" * 16
        self.assertNotEqual(hashlib.sha256(payload).hexdigest(), self.expected_sha)
        with patch_urlopen(payload)[0]:
            with self.assertRaises(cli.SasrsError) as ctx:
                cli._ensure_binary(cache_dir=self.cache)
        self.assertIn("SHA-256", str(ctx.exception))
        self.assertIn(self.expected_sha[:12], str(ctx.exception))
        self.assertFalse(self.binary().exists())
        self.assertEqual(list(self.cache.glob(cli.TEMP_PREFIX + "*")), [])


class TestLancementsConcurrents(WrapperTestCase):
    def test_deux_lancements_concurrents_ne_téléchargent_qu_une_fois(self):
        started = threading.Event()
        release = threading.Event()
        downloads = []

        def slow_urlopen(url, timeout=None):
            downloads.append(url)
            started.set()
            release.wait(5)  # ralentir le 1er téléchargement
            return FakeResponse(b"BINAIRE")

        erreurs = []
        resultats = []

        def launch():
            try:
                resultats.append(cli._ensure_binary(cache_dir=self.cache))
            except Exception as error:  # noqa: BLE001 - assert plus bas
                erreurs.append(error)

        with mock.patch.object(
            cli.urllib.request, "urlopen", side_effect=slow_urlopen
        ):
            first = threading.Thread(target=launch)
            first.start()
            self.assertTrue(started.wait(5), "le premier téléchargement n'a pas démarré")
            second = threading.Thread(target=launch)
            second.start()
            time.sleep(0.2)  # laisse le second thread atteindre le verrou
            release.set()
            first.join(10)
            second.join(10)

        self.assertEqual(erreurs, [])
        self.assertEqual(len(resultats), 2)
        self.assertEqual(resultats[0], resultats[1])
        # Un seul téléchargement : le second lancement a attendu le verrou
        # puis a constaté le marqueur vérifié.
        self.assertEqual(len(downloads), 1)
        self.assertTrue(cli._cache_is_verified(self.binary(), self.expected_sha))


class TestNettoyageTemporairesOrphelins(WrapperTestCase):
    def test_temporaire_orphelin_ancien_est_supprimé(self):
        self.cache.mkdir(parents=True, exist_ok=True)
        ancient = self.cache / (cli.TEMP_PREFIX + "abc123")
        ancient.write_bytes(b"partiel")
        recent = self.cache / (cli.TEMP_PREFIX + "def456")
        recent.write_bytes(b"en cours")
        vieux = time.time() - cli.ORPHAN_TEMP_MAX_AGE_S - 60
        os.utime(ancient, (vieux, vieux))
        cli._cleanup_orphan_temp(self.cache)
        self.assertFalse(ancient.exists())
        self.assertTrue(recent.exists())

    def test_cache_inexistant_ne_leve_pas(self):
        cli._cleanup_orphan_temp(self.cache / "inexistant")


class TestRepliWindowsOsReplace(WrapperTestCase):
    def test_echec_os_replace_avec_binaire_valide_en_place_replis_sur_lui(self):
        self.cache.mkdir(parents=True, exist_ok=True)
        self.binary().write_bytes(b"BINAIRE")  # déjà valide en place
        real_replace = os.replace

        def failing_replace(src, dst, **kwargs):
            if str(dst).endswith(ASSET):
                raise OSError(5, "Accès refusé (fichier verrouillé)")
            return real_replace(src, dst, **kwargs)

        with mock.patch.object(cli.os, "replace", side_effect=failing_replace):
            dest = cli._ensure_binary(cache_dir=self.cache)
        self.assertEqual(dest.read_bytes(), b"BINAIRE")
        self.assertTrue(cli._cache_is_verified(dest, self.expected_sha))

    def test_echec_os_replace_sans_binaire_valide_est_une_erreur_claire(self):
        self.cache.mkdir(parents=True, exist_ok=True)
        real_replace = os.replace

        def failing_replace(src, dst, **kwargs):
            if str(dst).endswith(ASSET):
                raise OSError(5, "Accès refusé")
            return real_replace(src, dst, **kwargs)

        with mock.patch.object(cli.os, "replace", side_effect=failing_replace):
            with patch_urlopen(b"BINAIRE")[0]:
                with self.assertRaises(cli.SasrsError) as ctx:
                    cli._ensure_binary(cache_dir=self.cache)
        self.assertIn("fermez les processus sasrs", str(ctx.exception))


if __name__ == "__main__":
    unittest.main()
