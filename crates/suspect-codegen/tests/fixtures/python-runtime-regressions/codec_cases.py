"""Expected-correct source-bound codec consumers, configured by public emitters."""
import json
from pathlib import Path
import sys
import unittest

import model_codecs as C
import models as M
from codec_runtime import CodecError

CONFIG = json.loads(Path("consumer-config.json").read_text())


class MixedUnions(unittest.TestCase):
    def test_decoded_object_can_be_encoded_after_a_literal_arm(self):
        codec = getattr(C, CONFIG.get("mixed_codec", "LiteralOrObjectCodec"))
        value = codec.decode('{"name":"ok"}')
        self.assertIsInstance(value, M.Config)
        self.assertEqual(codec.encode_value(value), {"name": "ok"})
        self.assertEqual(codec.decode(codec.encode(value)).name, "ok")
        self.assertEqual(codec.decode('"auto"'), "auto")
        self.assertEqual(codec.encode("auto"), '"auto"')

    def test_generic_json_arm_can_yield_to_the_native_object_arm(self):
        value = M.Config(name="ok")
        self.assertEqual(C.JsonOrObjectCodec.encode_value(value), {"name": "ok"})


class BranchResources(unittest.TestCase):
    def test_json_resource_failure_does_not_become_an_alternative_mismatch(self):
        # Rust supplies json_limits.max_work=32. The first literal trial cannot
        # clone this value within that policy. A later list arm could convert it
        # without JSON cloning, but may not erase the exhausted trial's outcome.
        with self.assertRaises(CodecError) as caught:
            C.LiteralOrStringsCodec.encode_value(["x" * 64])
        self.assertEqual(caught.exception.kind, "resource")
        # Every public call gets a fresh budget.
        self.assertEqual(C.LiteralOrStringsCodec.encode_value("auto"), "auto")


class SharedCopyBudget(unittest.TestCase):
    def setUp(self):
        self.model = getattr(M, CONFIG.get("copy_model", "CopyBox"))
        self.codec = getattr(C, CONFIG.get("copy_codec", "CopyBoxCodec"))

    def box(self, entries):
        value = self.model(name="a")
        for name, item in entries.items():
            value.set_extra(name, item)
        return value

    def test_a_large_generic_json_copy_exhausts_the_conversion_budget(self):
        # Rust emits max_conversion_steps=512, while the JSON byte/work limits
        # still admit this input. Thus the shared conversion budget must stop it.
        value = self.box({"large": "x" * 65536})
        with self.assertRaises(CodecError) as caught:
            self.codec.encode_value(value)
        self.assertEqual(caught.exception.kind, "resource")
        self.assertEqual(self.codec.encode_value(self.box({})), {"name": "a"})

    def test_json_copies_share_one_budget_across_extra_fields(self):
        # A single chunk comfortably fits; their aggregate exceeds the public
        # conversion policy even though every individual JSON clone would fit.
        chunk = "x" * CONFIG.get("copy_chunk_bytes", 64)
        count = CONFIG.get("copy_chunks", 16)
        self.assertEqual(self.codec.encode_value(self.box({"one": chunk}))["one"], chunk)
        value = self.box({f"extra{i}": chunk for i in range(count)})
        with self.assertRaises(CodecError) as caught:
            self.codec.encode_value(value)
        self.assertEqual(caught.exception.kind, "resource")
        self.assertEqual(self.codec.encode_value(self.box({"one": chunk}))["one"], chunk)

    def test_generic_json_descendants_are_charged_too(self):
        value = self.box({"nested": [None] * 1024})
        with self.assertRaises(CodecError) as caught:
            self.codec.encode_value(value)
        self.assertEqual(caught.exception.kind, "resource")


class LiteralEqualityBudget(unittest.TestCase):
    def test_literal_equality_exhaustion_is_a_located_codec_resource_error(self):
        # Rust emits schema.max_equality_steps=1. Root validation consumes one
        # comparison; conversion's source literal comparison must retain its
        # resource classification and valid source/instance identities.
        with self.assertRaises(CodecError) as caught:
            C.TaggedCodec.decode('{"kind":"tag","text":"ok"}')
        error = caught.exception
        self.assertEqual(error.kind, "resource")
        source = CONFIG["document"] + "#/components/schemas/Tagged"
        self.assertTrue(error.source == source or error.source.startswith(source + "/"), error.source)
        self.assertEqual(error.path, "/kind")


class IntegerConversion(unittest.TestCase):
    def test_wire_integer_with_padded_exponent_decodes_exactly(self):
        for padding in (41, 128):
            for token, expected in (
                ("10e-" + "0" * padding + "1", 1),
                ("0.1e+" + "0" * padding + "1", 1),
                ("-12500e-" + "0" * padding + "2", -125),
            ):
                with self.subTest(padding=padding, prefix=token[:12]):
                    value = C.IntegerBoxCodec.decode('{"count":' + token + '}')
                    self.assertIs(type(value.count), int)
                    self.assertEqual(value.count, expected)
                    self.assertEqual(C.IntegerBoxCodec.decode(C.IntegerBoxCodec.encode(value)).count, expected)


class OriginalContainerFile(unittest.TestCase):
    def test_original_openrouter_integer_field_keeps_its_exact_value(self):
        codec = getattr(C, CONFIG["container_file_codec"])
        for padding in (41, 128):
            token = "10e-" + "0" * padding + "1"
            text = (
                '{"bytes":' + token + ',"container_id":"c","created_at":1,'
                '"id":"f","object":"container.file","path":"p","source":"assistant"}'
            )
            with self.subTest(padding=padding):
                value = codec.decode(text)
                self.assertIs(type(value.bytes), int)
                self.assertEqual(value.bytes, 1)
                self.assertEqual(value.created_at, 1)
                self.assertEqual(codec.decode(codec.encode(value)).bytes, 1)


if __name__ == "__main__":
    print("NATIVE_PYTHON=" + json.dumps({"executable": sys.executable, "version": sys.version, "codecs": C.__file__}), flush=True)
    unittest.main()
