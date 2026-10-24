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
DEFINITIONS = (WORKER / 'src/automation_definitions.rs').read_text()
DEFINITION_SCHEMA = (WORKER / 'migrations/0036_automation_definitions.sql').read_text()

def definition_sql(name):
    return re.search(r'pub const ' + name + r':\s*&str\s*=\s*"(.*?)";', DEFINITIONS, re.S).group(1)

def sql(name):
    return re.search(r'pub const ' + name + r': &str = "(.*?)";', SOURCE, re.S).group(1)

class Journal(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.path = pathlib.Path(self.tmp.name) / 'journal.db'
        self.db = sqlite3.connect(self.path)
        self.db.executescript(SCHEMA)
        self.db.executescript((WORKER / 'migrations/0037_automation_execution.sql').read_text())

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

class DefinitionClock(Journal):
    def setUp(self):
        super().setUp()
        self.db.executescript(DEFINITION_SCHEMA)

    def definition(self, user='a', identity='routine', revision='rev', base=None, agent='agent'):
        return self.db.execute(definition_sql('WRITE_DEFINITION'),
            (user, identity, agent, '{}', revision, revision, '[1000]', 10, base)).rowcount

    def tick(self, revision='rev', previous='[1000]', run='scheduled', at=20):
        with self.db:
            inserted = self.db.execute(definition_sql('INSERT_SCHEDULED_RUN'),
                ('a', run, '{}', 'digest', at, 'routine', revision, previous)).rowcount
            advanced = self.db.execute(definition_sql('ADVANCE_CLOCK'),
                ('[2000]', 'a', 'routine', revision, previous, run, 'digest', at)).rowcount
        return inserted, advanced

    def test_clock_batch_duplicate_restart_and_stale_revision(self):
        self.definition()
        self.assertEqual(self.tick(), (1, 1))
        self.assertEqual(self.tick(), (0, 0))
        self.db.close()
        self.db = sqlite3.connect(self.path)
        self.assertEqual(self.tick(), (0, 0))
        self.assertEqual(self.tick(revision='stale', previous='[2000]', run='different'), (0, 0))
        self.assertEqual(self.db.execute('SELECT COUNT(*) FROM automation_runs').fetchone(), (1,))
        self.assertEqual(self.db.execute('SELECT next_runs_json,clock_checked_at,updated_at,revision FROM automation_definitions').fetchone(),
                         ('[2000]', 20, 10, 'rev'))

    def test_definition_cas_cannot_create_with_missing_base_or_move_agent(self):
        self.assertEqual(self.definition(base='missing'), 0)
        self.assertEqual(self.definition(), 1)
        self.assertEqual(self.definition(revision='new', base='stale'), 0)
        self.assertEqual(self.definition(revision='new', base='rev', agent='other'), 0)
        self.assertEqual(self.definition(revision='new', base='rev'), 1)
        self.assertEqual(self.tick(), (0, 0))

    def test_capacity_preserves_due_cursor_and_rotates_all_due_siblings(self):
        self.definition()
        self.definition(identity='sibling')
        self.definition(user='z')
        self.db.executemany('INSERT INTO automation_runs(user_id,run_uuid,event_json,event_digest,created_at) VALUES(?,?,?,?,?)',
            [('a', 'full-'+str(i), '{}', 'digest', 10) for i in range(20000)])
        self.db.commit()
        self.assertEqual(self.tick(), (0, 0))
        source = (WORKER / 'src/worker_api/automation_control.rs').read_text()
        rotate = re.search(r'"(UPDATE automation_definitions SET clock_checked_at=\?1.*?)"', source, re.S).group(1)
        self.db.execute(rotate, (30, 'a', 1000))
        select = re.search(r'"(SELECT d.user_id FROM automation_definitions d,json_each.*?)"', source, re.S).group(1)
        self.assertEqual(self.db.execute(select, (1000,)).fetchall(), [('z',), ('a',)])
        self.assertEqual(self.db.execute("SELECT next_runs_json,updated_at,revision FROM automation_definitions WHERE user_id='a'").fetchall(),
                         [('[1000]', 10, 'rev'), ('[1000]', 10, 'rev')])

if __name__ == '__main__':
    unittest.main()
