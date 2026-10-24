"""Exercise production journal SQL against real SQLite, including persistence."""
import pathlib
import re
import sqlite3
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
WORKER = ROOT / 'third_party/mahayana/mahayana-rs/mahayana-platform-worker'
SOURCE = (WORKER / 'src/automation_runs.rs').read_text()
SCHEMA = (WORKER / 'migrations/0035_automation_runs.sql').read_text()

def sql(name):
    return re.search(r'pub const ' + name + r': &str = "(.*?)";', SOURCE, re.S).group(1)

class Journal(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.tmp.name) / 'journal.db'
        self.db = sqlite3.connect(self.path)
        self.db.executescript(SCHEMA)

    def tearDown(self):
        self.db.close()
        self.tmp.cleanup()

    def create(self, user='a', run='r', event='{}', digest='event'):
        self.db.execute(sql('INSERT_RUN'), (user, run, event, digest, 100))

    def complete(self, user='a', run='r', value='{"status":"succeeded"}', digest='completion', at=110):
        return self.db.execute(sql('COMPLETE_RUN'), (value, digest, at, user, run)).rowcount

    def test_first_completion_wins_and_identical_retry_retains_receipt_inputs(self):
        self.create()
        self.assertEqual(self.complete(), 1)
        self.assertEqual(self.complete(at=999), 0)
        self.assertEqual(self.complete(value='{"status":"failed"}', digest='other'), 0)
        self.assertEqual(self.db.execute('SELECT completion_json,completion_digest,completed_at FROM automation_runs').fetchone(), ('{"status":"succeeded"}', 'completion', 110))

    def test_pending_ack_is_refused_and_account_scope_is_preserved(self):
        self.create('a')
        self.create('b')
        self.assertEqual(self.db.execute(sql('ACK_RUN'), (120, 'a', 'r')).rowcount, 0)
        self.assertEqual(self.complete('b'), 1)
        self.assertEqual(self.db.execute(sql('ACK_RUN'), (120, 'a', 'r')).rowcount, 0)
        self.assertEqual(self.db.execute(sql('ACK_RUN'), (120, 'b', 'r')).rowcount, 1)
        self.assertEqual(self.db.execute('SELECT user_id,acknowledged_at FROM automation_runs ORDER BY user_id').fetchall(), [('a', None), ('b', 120)])
        self.assertEqual(self.complete('foreign'), 0)

    def test_restart_and_ack_keep_tombstone_and_original_event(self):
        self.create()
        self.complete()
        self.db.execute(sql('ACK_RUN'), (120, 'a', 'r'))
        self.db.commit()
        self.db.close()
        self.db = sqlite3.connect(self.path)
        self.create(event='{"conflict":true}', digest='different-event')
        self.assertEqual(self.db.execute('SELECT event_json,event_digest,completion_digest,acknowledged_at FROM automation_runs').fetchone(), ('{}','event','completion',120))
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM automation_runs WHERE acknowledged_at IS NULL').fetchone(), (0,))

    def test_schema_rejects_partial_completion_or_ack_without_completion(self):
        self.create()
        for mutation in ["acknowledged_at=120", "completion_digest='x'", "completed_at=110", "completion_json='{}'", "completion_json='not-json',completion_digest='x',completed_at=110"]:
            with self.assertRaises(sqlite3.IntegrityError):
                self.db.execute('UPDATE automation_runs SET ' + mutation)

    def test_concurrent_duplicate_connections_keep_one_terminal_record(self):
        self.create()
        self.db.commit()
        other = sqlite3.connect(self.path)
        self.assertEqual(self.complete(), 1)
        self.db.commit()
        self.assertEqual(other.execute(sql('COMPLETE_RUN'), ('{"status":"failed"}', 'conflict', 200, 'a', 'r')).rowcount, 0)
        other.commit()
        other.close()
        self.assertEqual(self.db.execute('SELECT COUNT(*),completion_digest FROM automation_runs').fetchone(), (1,'completion'))

if __name__ == '__main__':
    unittest.main()
