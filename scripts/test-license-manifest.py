#!/usr/bin/env python3
"""Private source/package fixtures; no user desktop or live socket is touched."""
import importlib.util
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.dont_write_bytecode = True

SPEC=importlib.util.spec_from_file_location('records',Path(__file__).with_name('license-manifest.py'))
records=importlib.util.module_from_spec(SPEC);SPEC.loader.exec_module(records)
FIXTURE_SPEC=importlib.util.spec_from_file_location('performance_fixture',Path(__file__).with_name('performance-test-support.py'))
performance_fixture=importlib.util.module_from_spec(FIXTURE_SPEC);FIXTURE_SPEC.loader.exec_module(performance_fixture)
ROOT=Path(__file__).resolve().parent.parent
class RecordsTests(unittest.TestCase):
    def setUp(self):
        self.temp=tempfile.TemporaryDirectory(prefix='omatainer-license-test-');self.addCleanup(self.temp.cleanup)
        self.base=Path(self.temp.name);self.root=self.base/'source';self.root.mkdir()
        manifest=records.load(ROOT/'licenses/manifest.json')
        for name in [*manifest['source_files'],'licenses/manifest.json','licenses/notices.json']:
            out=self.root/name;out.parent.mkdir(parents=True,exist_ok=True);shutil.copy2(ROOT/name,out)
        self.meta=records.metadata(self.root)
        # Tiny native ELF fixture preserves the production record-dump protocol.
        source=self.base/'fixture.c';self.binary=self.base/'fixture'
        source.write_text('#include <stdio.h>\n#include <string.h>\nint main(int argc,char**argv){if(argc>1 && !strcmp(argv[1],"omatainer:offline-license-records:v1"))return 7;if(argc!=3)return 1;const char*p=NULL;'+
            'if(!strcmp(argv[2],"--manifest"))p='+json.dumps(str(self.root/'licenses/manifest.json'))+';'+
            'if(!strcmp(argv[2],"--notices"))p='+json.dumps(str(self.root/'licenses/notices.json'))+';'+
            'if(!p)return 2;FILE*f=fopen(p,"rb");if(!f)return 3;int c;while((c=fgetc(f))!=EOF)putchar(c);fclose(f);return 0;}')
        subprocess.run(['cc',str(source),'-o',str(self.binary)],check=True,capture_output=True)
        performance_fixture.report(self.root,self.binary)
    def package(self):
        return records.package(self.root,self.binary,self.base/'release',self.meta)
    def test_exact_inventory_and_notices_survive_release_reopen(self):
        doc=records.validate(self.root,self.meta)
        output=self.package();receipt=records.verify_package(output)
        self.assertEqual(len(receipt['files']),len(doc['package'])+4)
        self.assertEqual((output/records.LICENSE_ROOT/'notices.json').read_bytes(),(self.root/'licenses/notices.json').read_bytes())
        self.assertEqual(receipt['source_files'],doc['source_files'])
        self.assertTrue(all(entry['notices'] for entry in doc['entries']))
        with self.assertRaisesRegex(records.ManifestError,'already exists'):self.package()
    def test_missing_extra_tampered_source_or_notice_refuses_publication(self):
        for change in ['extra','missing','altered','notice','rust','script','build','dependency','fixture','embedded','manual','vendor','extra_vendor']:
            with self.subTest(change=change):
                path=None;before=None
                if change=='extra':path=self.root/'plugin/unlicensed.wav'
                elif change=='missing':path=self.root/'plugin/Service.qml';before=path.read_bytes();path.unlink()
                elif change=='altered':path=self.root/'contrib/omatainer.lua';before=path.read_bytes();path.write_bytes(before+b'\n')
                elif change=='notice':path=self.root/'licenses/notices.json';before=path.read_bytes();path.write_text('{}')
                elif change=='rust':path=self.root/'src/new_unreviewed_asset.rs'
                elif change=='script':path=self.root/'scripts/unreviewed.sh'
                elif change=='build':path=self.root/'build.rs'
                elif change=='dependency':path=self.root/'Cargo.lock';before=path.read_bytes();path.write_bytes(before+b'\n')
                elif change=='fixture':path=self.root/'tests/new-unreviewed.wav'
                elif change=='embedded':path=self.root/'src/new-unreviewed.bin'
                elif change=='vendor':path=self.root/'vendor/symphonia-format-riff/src/aiff/chunks.rs';before=path.read_bytes();path.write_bytes(before+b'\n')
                elif change=='extra_vendor':path=self.root/'vendor/symphonia-format-riff/src/unreviewed.rs'
                elif change=='manual':path=self.root/'docs/manual.md';before=path.read_bytes();path.write_bytes(before+b'\n')
                if change in ['extra','rust','script','build','fixture','embedded','extra_vendor']:path.write_text('unreviewed')
                with self.assertRaises((records.ManifestError,KeyError)):self.package()
                self.assertFalse((self.base/'release').exists())
                if before is None:path.unlink()
                else:path.write_bytes(before)
    def test_package_extra_missing_altered_files_fail_recheck(self):
        output=self.package()
        extra=output/'rogue-model.bin';extra.write_bytes(b'x')
        with self.assertRaisesRegex(records.ManifestError,'unmanifested'):records.verify_package(output)
        extra.unlink();path=output/records.LICENSE_ROOT/'notices.json';before=path.read_bytes();path.unlink()
        with self.assertRaisesRegex(records.ManifestError,'missing'):records.verify_package(output)
        path.write_bytes(before+b' ')
        with self.assertRaisesRegex(records.ManifestError,'hash mismatch'):records.verify_package(output)
    def test_cancellation_and_racing_destination_leave_prior_content(self):
        with patch.object(records,'verify_package',side_effect=KeyboardInterrupt):
            with self.assertRaises(KeyboardInterrupt):self.package()
        self.assertFalse((self.base/'release').exists())
        self.assertEqual(list(self.base.glob('.omatainer-package-*')),[])
        original=records.publish_directory
        def race(source,destination):
            destination.mkdir();(destination/'mine').write_text('keep')
            original(source,destination)
        with patch.object(records,'publish_directory',side_effect=race):
            with self.assertRaises(OSError):self.package()
        self.assertEqual((self.base/'release/mine').read_text(),'keep')
    def test_stale_binary_records_and_cargo_features_are_rejected(self):
        with patch.object(records.subprocess,'run',return_value=subprocess.CompletedProcess([],0,b'{}',b'')):
            with self.assertRaisesRegex(records.ManifestError,'different license records'):records.verify_binary(self.root,self.binary)
        old=self.base/'old-binary'
        marker=b'omatainer:offline-license-records:v1'
        old.write_bytes(self.binary.read_bytes().replace(marker,b'x'*len(marker)))
        with patch.object(records.subprocess,'run',side_effect=AssertionError('old executable must not be launched')):
            with self.assertRaisesRegex(records.ManifestError,'predates'):records.verify_binary(self.root,old)
        changed=json.loads(json.dumps(self.meta));changed['resolve']['nodes'][0]['features'].append('unreviewed')
        with self.assertRaisesRegex(records.ManifestError,'Cargo components'):records.validate(self.root,changed)
    def test_standalone_verification_needs_no_toolchain_and_font_upgrade_needs_review(self):
        output=self.package()
        with patch.dict('os.environ', {'PATH':''}):
            result=subprocess.run([sys.executable,str(ROOT/'scripts/license-manifest.py'),
                                   'verify-package','--destination',str(output)],capture_output=True,text=True)
        self.assertEqual(result.returncode,0,result.stderr)
        changed=json.loads(json.dumps(self.meta))
        next(p for p in changed['packages'] if p['name']=='epaint_default_fonts')['version']='99.0.0'
        # The explicit policy guard runs independently before a new font version
        # can be associated with old per-face terms or version strings.
        with self.assertRaisesRegex(records.ManifestError,'font dependency changed'):
            records.update(self.root,changed)
        with patch.object(records.subprocess,'run',return_value=subprocess.CompletedProcess([],0,'host: unsupported-platform\n','')):
            with self.assertRaisesRegex(records.ManifestError,'supports native Linux'):records.toolchain()

    def test_performance_evidence_is_mandatory_and_raw_values_are_rechecked_after_packaging(self):
        report=self.root/records.gate.REPORT
        original=report.read_bytes();report.unlink()
        with self.assertRaisesRegex(ValueError,'regular performance'):self.package()
        self.assertFalse((self.base/'release').exists())
        report.write_bytes(original)
        output=self.package();path=output/records.gate.INSTALLED
        value=json.loads(path.read_text());value['raw']['workloads'][0]['measurements'][0]['samples']['frame_wall_ns'][0]=999
        path.write_bytes(records.encoded(value))
        receipt_path=output/records.RECEIPT;receipt=json.loads(receipt_path.read_text())
        receipt['files'][records.gate.INSTALLED]=records.sha(path.read_bytes())
        receipt_path.write_bytes(records.encoded(receipt))
        # Rehashing the outer file cannot turn false timing summaries into proof.
        with self.assertRaisesRegex(ValueError,'raw evidence'):records.verify_package(output)

    def test_known_mpl_and_font_terms_are_retained_with_primary_sources(self):
        manifest=records.load(self.root/'licenses/manifest.json');notes=records.load(self.root/'licenses/notices.json')
        for id,expected in [('font:hack','Bitstream'),('font:ubuntu','UBUNTU FONT LICENCE'),('font:noto-emoji','SIL OPEN FONT LICENSE'),('font:emoji-icon','John Slegers'),('crate:symphonia@0.5.5','Mozilla Public License')]:
            entry=next(e for e in manifest['entries'] if e['id']==id)
            self.assertIn(expected,'\n'.join(notes[r['sha256']] for r in entry['notices']))
            self.assertTrue(entry['commercial_use']);self.assertTrue(entry['redistribution'])
        self.assertTrue(any(e['category']=='toolchain-runtime' for e in manifest['entries']))
if __name__=='__main__':unittest.main()
