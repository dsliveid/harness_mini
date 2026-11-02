import sqlite3
import json
import sys

sys.stdout.reconfigure(encoding='utf-8')
conn = sqlite3.connect(r'F:\Software\HarnessMini\data\harness_mini.db')
cursor = conn.cursor()

child_id = 'af487171-ddb5-47bc-a54d-6d472066524d'

cursor.execute("SELECT payload_json FROM llm_requests WHERE session_id = ? ORDER BY id ASC LIMIT 1", (child_id,))
row = cursor.fetchone()
if row:
    payload = json.loads(row[0])
    print("=== SYSTEM PROMPT ===")
    msgs = payload.get("messages", [])
    if msgs:
        sys_prompt = msgs[0].get("content", "")
        print(f"Length of System Prompt: {len(sys_prompt)} chars")
        print(sys_prompt)
        
        print("\n=== USER MESSAGE (First) ===")
        if len(msgs) > 1:
            user_msg = msgs[1].get("content", "")
            print(f"Length of User Message: {len(user_msg)} chars")
            print(user_msg)
            
    print("\n=== TOOLS PASSED TO MODEL ===")
    tools = payload.get("tools", [])
    print(f"Total tools count: {len(tools)}")
    tool_names = [t.get("function", {}).get("name") for t in tools]
    print("Tool names:", tool_names)
