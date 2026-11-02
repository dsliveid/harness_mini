import sqlite3
import json
import sys

sys.stdout.reconfigure(encoding='utf-8')
conn = sqlite3.connect(r'F:\Software\HarnessMini\data\harness_mini.db')
cursor = conn.cursor()

child_id = 'af487171-ddb5-47bc-a54d-6d472066524d'

cursor.execute("SELECT payload_json FROM llm_requests WHERE session_id = ? ORDER BY id ASC LIMIT 1", (child_id,))
row = cursor.fetchone()
payload = json.loads(row[0])
sys_prompt = payload["messages"][0]["content"]

lines = sys_prompt.split("\n")
for i in range(50, 95):
    if i < len(lines):
        print(f"L{i}: {lines[i]}")
