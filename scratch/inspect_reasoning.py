import sqlite3
import json
import sys

sys.stdout.reconfigure(encoding='utf-8')
conn = sqlite3.connect(r'F:\Software\HarnessMini\data\harness_mini.db')
cursor = conn.cursor()

child_id = 'af487171-ddb5-47bc-a54d-6d472066524d'

cursor.execute("SELECT seq, reasoning FROM messages WHERE session_id = ? AND reasoning IS NOT NULL AND reasoning != '' ORDER BY seq ASC", (child_id,))
rows = cursor.fetchall()

print(f"Total reasoning messages: {len(rows)}")
# Sample across the trajectory
indices = [0, 5, 10, 15, 20, 25, 30, len(rows)-3, len(rows)-1]
for idx in indices:
    if idx < len(rows):
        r = rows[idx]
        print(f"\n--- Turn seq={r[0]} ---")
        text = r[1]
        print(text[:400] + "..." if len(text) > 400 else text)
