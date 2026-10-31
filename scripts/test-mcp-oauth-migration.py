import pathlib, sqlite3, unittest
ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA = ROOT / 'third_party/mahayana/mahayana-rs/mahayana-platform-worker/account-migrations/0006_mcp_connection_oauth.sql'
class Migration(unittest.TestCase):
    def setUp(self):
        self.db=sqlite3.connect(':memory:')
        self.db.execute('PRAGMA foreign_keys=ON')
        self.db.execute('CREATE TABLE account_connections (connection_id TEXT PRIMARY KEY)')
        self.db.executescript(SCHEMA.read_text())
        self.db.executescript(SCHEMA.read_text())
        self.db.execute("INSERT INTO account_mcp_oauth_attempts VALUES ('a','user-a','session-a','fabushi-official-github','hash','ticket','encrypted-verifier','pending',NULL,100,700)")
    def test_callback_is_single_use_and_expiry_bound(self):
        query="UPDATE account_mcp_oauth_attempts SET status='exchanging' WHERE state_hash=? AND status='pending' AND expires_at>? RETURNING attempt_id"
        self.assertEqual(self.db.execute(query,('hash',200)).fetchall(),[('a',)])
        self.assertEqual(self.db.execute(query,('hash',200)).fetchall(),[])
        self.db.execute("UPDATE account_mcp_oauth_attempts SET status='pending'")
        self.assertEqual(self.db.execute(query,('hash',700)).fetchall(),[])
    def test_poll_requires_both_account_and_session(self):
        query='SELECT attempt_id FROM account_mcp_oauth_attempts WHERE attempt_id=? AND user_id=? AND session_id=?'
        self.assertEqual(self.db.execute(query,('a','user-b','session-a')).fetchall(),[])
        self.assertEqual(self.db.execute(query,('a','user-a','session-b')).fetchall(),[])
        self.assertEqual(self.db.execute(query,('a','user-a','session-a')).fetchall(),[('a',)])
    def test_cancellation_wins_over_late_delivery(self):
        self.db.execute("UPDATE account_mcp_oauth_attempts SET status='cancelled',payload_ciphertext=NULL,verifier='' WHERE attempt_id='a'")
        self.assertEqual(self.db.execute("UPDATE account_mcp_oauth_attempts SET status='ready',payload_ciphertext='cipher' WHERE attempt_id='a' AND status='exchanging' RETURNING attempt_id").fetchall(),[])
        self.assertEqual(self.db.execute('SELECT payload_ciphertext,verifier FROM account_mcp_oauth_attempts').fetchone(),(None,''))
    def test_ack_erases_delivery_and_schema_rejects_invalid_state_and_orphan_credentials(self):
        self.db.execute("UPDATE account_mcp_oauth_attempts SET status='ready',payload_ciphertext='cipher'")
        self.db.execute("UPDATE account_mcp_oauth_attempts SET status='consumed',payload_ciphertext=NULL,verifier='' WHERE status='ready'")
        self.assertEqual(self.db.execute('SELECT payload_ciphertext FROM account_mcp_oauth_attempts').fetchone(),(None,))
        with self.assertRaises(sqlite3.IntegrityError): self.db.execute("UPDATE account_mcp_oauth_attempts SET status='connected'")
        with self.assertRaises(sqlite3.IntegrityError): self.db.execute("INSERT INTO account_mcp_native_credentials VALUES ('missing','a','plugin','digest',NULL,1)")
    def test_cleanup_retains_unprocessed_ready_grants(self):
        self.db.execute("UPDATE account_mcp_oauth_attempts SET status='ready',payload_ciphertext='cipher'")
        for n in range(150):
            self.db.execute("INSERT INTO account_mcp_oauth_attempts VALUES (?,?,?,?,?,?,?,'ready','cipher',100,700)",(str(n),'user-a','session-a','plugin',f'state-{n}',f'ticket-{n}','encrypted'))
        processed=self.db.execute("SELECT attempt_id FROM account_mcp_oauth_attempts WHERE expires_at<=800 AND status='ready' LIMIT 100").fetchall()
        for (attempt,) in processed:
            self.db.execute("UPDATE account_mcp_oauth_attempts SET status='expired',payload_ciphertext=NULL,verifier='' WHERE attempt_id=? AND status='ready' AND expires_at<=800",(attempt,))
        self.db.execute("UPDATE account_mcp_oauth_attempts SET status='expired',payload_ciphertext=NULL,verifier='' WHERE expires_at<=800 AND status NOT IN ('ready','expired','cancelled','consumed')")
        self.db.execute("DELETE FROM account_mcp_oauth_attempts WHERE expires_at<1000 AND status!='ready'")
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM account_mcp_oauth_attempts WHERE status='ready' AND payload_ciphertext='cipher'").fetchone(),(51,))
        self.assertEqual(self.db.execute("SELECT COUNT(*) FROM account_mcp_oauth_attempts").fetchone(),(51,))
if __name__=='__main__': unittest.main()

