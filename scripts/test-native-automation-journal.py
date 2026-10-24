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

class Execution(Journal):
    def setUp(self):
        super().setUp()
        self.db.executescript(DEFINITION_SCHEMA)
        self.db.executescript((WORKER / 'migrations/0006_remote_computer.sql').read_text())
        self.db.executemany('INSERT INTO remote_computers(device_id,user_id,label,device_secret_hash,created_at,last_seen_at) VALUES(?,?,?,?,?,?)',
            [('device-a','a','A','hash-a',1,1), ('device-b','a','B','hash-b',1,1)])
        self.create()
        self.db.commit()

    def statement(self, name):
        source = (WORKER / 'src/automation_execution.rs').read_text()
        return re.search(r'pub const ' + name + r':\s*&str\s*=\s*"(.*?)";', source, re.S).group(1)

    def claim(self, claim='claim-a', token='token-a', device='device-a', now=100, db=None):
        return (db or self.db).execute(self.statement('CLAIM'),
            (claim,token,device,'session',now+60,'a','r',now)).rowcount

    def start(self, claim='claim-a', token='token-a', device='device-a', generation=1, now=110):
        return self.db.execute(self.statement('START'), (now,'a','r',claim,token,device,generation)).rowcount

    def test_device_race_expired_reservation_and_generation_fence(self):
        other=sqlite3.connect(self.path)
        self.assertEqual(self.claim(),1);self.db.commit()
        self.assertEqual(self.claim('claim-b','token-b','device-b',db=other),0);other.commit()
        self.assertEqual(self.claim(now=170),0,'same claim never refreshes even after expiry')
        self.assertEqual(self.claim('claim-b','token-b','device-b',now=170),1)
        self.assertEqual(self.start(),0)
        self.assertEqual(self.start('claim-b','token-b','device-b',generation=1,now=180),0)
        self.assertEqual(self.start('claim-b','token-b','device-b',generation=2,now=180),1)
        self.assertEqual(self.claim('claim-c','token-c','device-a',now=10000),0)
        other.close()

    def test_running_unknown_and_terminal_never_reclaim_or_accept_ordinary_completion(self):
        self.assertEqual(self.claim(),1);self.assertEqual(self.start(),1)
        self.assertEqual(self.complete(),0)
        self.assertEqual(self.db.execute(self.statement('UNKNOWN'),('a','r','claim-a','token-a','device-a',1)).rowcount,1)
        self.assertEqual(self.claim('claim-b','token-b','device-b',now=10000),0)
        args=('{}','terminal',200,'a','r','claim-a','token-a','device-a',1)
        self.assertEqual(self.db.execute(self.statement('COMPLETE'),args).rowcount,1)
        self.assertEqual(self.db.execute(self.statement('COMPLETE'),args).rowcount,0)
        self.assertEqual(self.claim('claim-b','token-b','device-b',now=10000),0)
        self.assertEqual(self.db.execute('SELECT completed_at,completion_digest FROM automation_runs').fetchone(),(200,'terminal'))

    def test_revoked_device_and_changed_definition_cannot_start(self):
        self.assertEqual(self.claim(),1)
        self.db.execute("UPDATE remote_computers SET revoked_at=105 WHERE device_id='device-a'")
        self.assertEqual(self.start(),0)
        self.db.execute("UPDATE remote_computers SET revoked_at=NULL WHERE device_id='device-a'")
        self.db.execute("UPDATE automation_runs SET event_json=? WHERE run_uuid='r'",
            ('{"definitionRevision":"rev","automationId":"routine","sandAgentId":"agent"}',))
        self.assertEqual(self.start(),0,'missing definition fences reserved start')
        self.db.execute("INSERT INTO automation_definitions(user_id,automation_id,agent_id,definition_json,revision,mutation_id,next_runs_json,updated_at) VALUES('a','routine','agent','{\"enabled\":true}','rev','mutation','[]',1)")
        self.assertEqual(self.start(),1)

if __name__ == '__main__':
    unittest.main()
