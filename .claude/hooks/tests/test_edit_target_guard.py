import json
from pathlib import Path
import subprocess
import tempfile
import unittest

GUARD = Path(__file__).resolve().parents[1] / 'edit-target-guard.py'


class EditTargetGuardTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name).resolve()
        self.feature = self.repo('feature', 'feat/work')
        self.protected = self.repo('protected', 'main')

    def repo(self, name, branch):
        path = self.root / name
        subprocess.run(['git', 'init', '-q', '-b', branch, str(path)], check=True)
        return path

    def invoke(self, tool_input, expected, cwd=None, engine=None):
        command = ['python3', str(GUARD)] if engine is None else ['bash', str(GUARD.parents[2] / f'.{engine}/hooks/implement-guard.sh')]
        result = subprocess.run(command, input=json.dumps({'cwd': str(cwd or self.feature), 'tool_input': tool_input}), text=True, capture_output=True, cwd=self.feature)
        self.assertEqual(result.returncode, expected, result.stderr)

    def patch(self, body, expected):
        self.invoke({'command': '*** Begin Patch\n' + body + '\n*** End Patch'}, expected)

    def test_claude_paths_and_native_wrappers(self):
        for engine in ['claude']:
            with self.subTest(engine=engine):
                self.invoke({'file_path': 'file.py'}, 0, engine=engine)
                self.invoke({'file_path': str(self.protected / 'file.py')}, 2, engine=engine)

    def test_patch_across_repositories(self):
        self.patch('*** Add File: file.py\n+ok', 0)
        self.patch(f'*** Add File: file.py\n+ok\n*** Update File: {self.protected}/file.py\n@@\n-x\n+y', 2)

    def test_move_destination(self):
        self.patch(f'*** Update File: old.py\n*** Move to: {self.protected}/new.py\n@@\n-x\n+y', 2)

    def test_nonexistent_parent_and_relative_cwd(self):
        self.invoke({'file_path': 'new/deep/file.py'}, 2, cwd=self.protected)
        self.invoke({'file_path': '../protected/new/deep/file.py'}, 2)
        self.invoke({'file_path': 'new/deep/file.py'}, 0)

    def test_symlink_destination(self):
        (self.feature / 'linked').symlink_to(self.protected, target_is_directory=True)
        self.invoke({'file_path': 'linked/new/file.py'}, 2)
        (self.protected / 'real.py').write_text('x')
        (self.feature / 'link.py').symlink_to(self.protected / 'real.py')
        self.patch('*** Update File: link.py\n@@\n-x\n+y', 2)

    def test_malformed_payload_does_not_pass(self):
        for payload in [{}, {'command': 'echo hello'}, {'command': '*** Begin Patch\n*** End Patch'}, {'file_path': 10}]:
            with self.subTest(payload=payload):
                self.invoke(payload, 2)

    def test_master_and_local_non_repository(self):
        master = self.repo('legacy', 'master')
        self.invoke({'file_path': str(master / 'file.py')}, 2)
        self.invoke({'file_path': str(self.root / 'local/file.py')}, 0)

    def test_delete_patch(self):
        self.patch(f'*** Delete File: {self.protected}/file.py', 2)


if __name__ == '__main__':
    unittest.main()
