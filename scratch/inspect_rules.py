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
print(f"Total lines in System Prompt: {len(lines)}")
for i, l in enumerate(lines):
    if any(k in l for k in ["工作规则", "SOP", "准则", "守则", "门禁", "边读边记", "方案先行", "精益代码", "安全红线", "扩展工具"]):
        print(f"L{i}: {l}")
