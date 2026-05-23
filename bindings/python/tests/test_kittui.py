import ctypes
import json
import unittest

from kittui import Kittui, KittuiError
from kittui.__main__ import build_parser


class FakeLib:
    def __init__(self):
        self.calls = []
        self.last_error = b"fake error"

    def kittui_runtime_new(self, cache_dir):
        self.calls.append(("new", cache_dir))
        return ctypes.c_void_p(0x1234)

    def kittui_runtime_new_config(self, config):
        self.calls.append(("new_config", json.loads(config.decode())))
        return ctypes.c_void_p(0x1234)

    def kittui_runtime_free(self, runtime):
        self.calls.append(("free", bool(runtime)))

    def kittui_string_free(self, ptr):
        self.calls.append(("free_string", bool(ptr)))

    def kittui_bytes_free(self, ptr, length):
        self.calls.append(("free_bytes", bool(ptr), int(length.value if hasattr(length, "value") else length)))

    def kittui_abi_version(self):
        return (0 << 16) | 7

    def kittui_probe_json(self, runtime):
        return b'{"abi_major":0,"abi_minor":7,"transport":"Direct"}'

    def kittui_unplace(self, runtime, image_id):
        self.calls.append(("unplace", int(image_id.value if hasattr(image_id, "value") else image_id)))
        return b"deleted"

    def kittui_last_error(self, runtime):
        return self.last_error

    def kittui_render_json(self, runtime, scene_json, out_ptr, out_len):
        self.calls.append(("render", json.loads(scene_json.decode())))
        data = ctypes.create_string_buffer(b"\x89PNGfake")
        self._render_buffer = data
        out_ptr._obj.contents = ctypes.cast(data, ctypes.POINTER(ctypes.c_uint8)).contents
        out_len._obj.value = len(data.raw) - 1
        return 0

    def kittui_place_json(self, runtime, scene_json, out):
        self.calls.append(("place", json.loads(scene_json.decode())))
        out._obj.value = b"placed"
        return 0

    def kittui_place_json_at(self, runtime, scene_json, x, y, out):
        self.calls.append(("place_at", json.loads(scene_json.decode()), int(x.value), int(y.value)))
        out._obj.value = b"placed-at"
        return 0

    def kittui_place_many_json(self, runtime, scenes_json, out):
        self.calls.append(("place_many", json.loads(scenes_json.decode())))
        out._obj.value = b"placed-many"
        return 0

    def kittui_place_many_json_at(self, runtime, scenes_json, x, y, out):
        self.calls.append(("place_many_at", json.loads(scenes_json.decode()), int(x.value), int(y.value)))
        out._obj.value = b"placed-many-at"
        return 0

    def kittui_place_many_json_channels(self, runtime, scenes_json, x, y, out):
        self.calls.append(("place_many_channels", json.loads(scenes_json.decode()), int(x.value), int(y.value)))
        out._obj.value = json.dumps({"count": 2, "upload": "u", "placement": "p", "embed": "e"}).encode()
        return 0


class FailingLib(FakeLib):
    def kittui_place_json(self, runtime, scene_json, out):
        return 3


class FailingRenderLib(FakeLib):
    def kittui_render_json(self, runtime, scene_json, out_ptr, out_len):
        return 3


SCENE = {
    "footprint": {"x": 0, "y": 0, "cols": 2, "rows": 1},
    "cell_size": {"width_px": 8, "height_px": 16},
    "layers": [],
}


class KittuiBindingTests(unittest.TestCase):
    def test_module_parser_accepts_discovery_flags(self):
        parser = build_parser()
        args = parser.parse_args(["--find-library"])
        self.assertTrue(args.find_library)
        args = parser.parse_args(["--abi", "--config-json", '{"renderer":"cpu"}'])
        self.assertTrue(args.abi)
        self.assertEqual(args.config_json, '{"renderer":"cpu"}')

    def test_config_probe_unplace_and_close(self):
        lib = FakeLib()
        k = Kittui.from_library(lib, {"cache_dir": "/tmp/kittui", "renderer": "cpu"})
        self.assertEqual(lib.calls[0], ("new_config", {"cache_dir": "/tmp/kittui", "renderer": "cpu"}))
        self.assertEqual(k.abi_version(), {"major": 0, "minor": 7})
        self.assertEqual(k.probe()["transport"], "Direct")
        self.assertEqual(k.unplace("0x10"), "deleted")
        k.close()
        self.assertEqual(lib.calls[-1][0], "free")

    def test_place_variants_normalize_dicts_and_json_strings(self):
        lib = FakeLib()
        k = Kittui.from_library(lib)
        self.assertEqual(k.render(SCENE), b"\x89PNGfake")
        self.assertTrue(any(call[0] == "free_bytes" for call in lib.calls))
        self.assertEqual(k.place(SCENE), "placed")
        self.assertEqual(k.place_at(SCENE, 7, 9), "placed-at")
        self.assertEqual(k.place_many([SCENE, json.dumps(SCENE)]), "placed-many")
        self.assertEqual(k.place_many_at([SCENE], 10, 20), "placed-many-at")
        channels = k.place_many_channels([SCENE, json.dumps(SCENE)], 3, 4)
        self.assertEqual(channels["upload"], "u")
        call = next(call for call in reversed(lib.calls) if call[0] == "place_many_channels")
        self.assertEqual(call[2:], (3, 4))
        k.close()

    def test_errors_include_last_error(self):
        k = Kittui.from_library(FailingLib())
        with self.assertRaisesRegex(KittuiError, "fake error"):
            k.place(SCENE)

    def test_render_errors_include_last_error(self):
        k = Kittui.from_library(FailingRenderLib())
        with self.assertRaisesRegex(KittuiError, "kittui_render_json.*fake error"):
            k.render(SCENE)


if __name__ == "__main__":
    unittest.main()
