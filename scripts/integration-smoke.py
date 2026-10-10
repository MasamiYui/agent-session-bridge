#!/usr/bin/env python3
"""Exercise actual installed CLIs with isolated stores and a local mock model."""
import json, os, pathlib, shutil, socket, subprocess, sys, tempfile, time
REPO = pathlib.Path(__file__).resolve().parent.parent
claude = shutil.which('claude')
codex = shutil.which('codex') or '/Applications/ChatGPT.app/Contents/Resources/codex-cli/CodexCLI.app/Contents/MacOS/codex'
if not claude or not pathlib.Path(codex).is_file():
    sys.exit('Install Codex 0.160.1 and Claude Code 2.1.280 to run the offline integration test.')
root = pathlib.Path(tempfile.mkdtemp(prefix='asb-integration-')).resolve()
with socket.socket() as sock:
    sock.bind(('127.0.0.1', 0)); port = sock.getsockname()[1]
server = subprocess.Popen([sys.executable,str(REPO/'scripts/mock-model-server.py'),'--port',str(port),'--log-dir',str(root)],stdout=subprocess.DEVNULL,stderr=subprocess.PIPE)
try:
    deadline = time.monotonic()+10
    while True:
        try:
            with socket.create_connection(('127.0.0.1',port),timeout=.2): break
        except OSError:
            if time.monotonic()>deadline: raise RuntimeError('Mock server did not start')
            time.sleep(.05)
    env = os.environ.copy()
    # The probe resolves installed CLIs; configuration, transcripts and indexes live only here.
    probe = subprocess.run(['cargo','run','--manifest-path','src-tauri/Cargo.toml','--no-default-features','--example','probe','--',str(root),str(port)],cwd=REPO,env=env,capture_output=True,text=True,timeout=180)
    (root/'probe.log').write_text(probe.stdout+'\n'+probe.stderr)
    if probe.returncode: raise RuntimeError('Codex migration probe failed; see probe.log')
    results = json.loads((root/'results.json').read_text())
    request = json.loads((root/'codex-request.json').read_text())
    assert 'BRIDGE_ORCHID' in json.dumps(request['input']), 'Codex failed to include imported history'
    env.update(CLAUDE_CONFIG_DIR=str(root/'claude'),ANTHROPIC_BASE_URL=f'http://127.0.0.1:{port}',ANTHROPIC_API_KEY='asb-offline-test-key',CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC='1')
    for key in ['ANTHROPIC_AUTH_TOKEN','CLAUDE_CODE_OAUTH_TOKEN','CLAUDE_CODE_USE_BEDROCK','CLAUDE_CODE_USE_VERTEX','CLAUDE_CODE_USE_FOUNDRY']:
        env.pop(key,None)
    cmd = [claude,'--bare','--resume',results['claude']['targetId'],'--print','--model','mock-model','--tools','','--strict-mcp-config','--mcp-config','{"mcpServers":{}}','--output-format','json','What codeword did we agree on?']
    out = subprocess.run(cmd,cwd=root/'project',env=env,capture_output=True,text=True,timeout=60)
    (root/'claude-output.json').write_text(out.stdout);(root/'claude-stderr.log').write_text(out.stderr)
    if out.returncode: raise RuntimeError('Claude resume failed: '+out.stderr[:1000]+out.stdout[:1000])
    request = json.loads((root/'claude-request.json').read_text())
    assert 'BRIDGE_ORCHID' in json.dumps(request['messages'][:-1]), 'Claude failed to include imported history'
    assert 'BRIDGE_ORCHID' in out.stdout, 'Claude did not complete the resumed turn'
    print('PASS: Claude → Codex → Claude; both actual CLIs sent imported history to the offline model and continued. Evidence: '+str(root))
finally:
    server.terminate()
    try: server.wait(timeout=5)
    except subprocess.TimeoutExpired: server.kill();server.wait()
    print('Isolated evidence: '+str(root))
