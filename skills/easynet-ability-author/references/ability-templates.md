# Ability Templates

Ready-to-use ability.json templates for common use cases.

## System Monitoring

### CPU & Memory

```json
{
  "name": "system-metrics",
  "version": "1.0.0",
  "tool_name": "system-metrics",
  "description": "Collect CPU load, memory usage, and process count",
  "command": "python3 -c \"import json,os,subprocess; m=subprocess.run(['free','-m'],capture_output=True,text=True); lines=m.stdout.strip().split('\\n'); mem=lines[1].split(); print(json.dumps({'load_1m':os.getloadavg()[0],'mem_total_mb':int(mem[1]),'mem_used_mb':int(mem[2]),'mem_free_mb':int(mem[3]),'processes':len(os.listdir('/proc'))//2}))\""
}
```

### Disk Usage

```json
{
  "name": "disk-usage",
  "version": "1.0.0",
  "tool_name": "disk-usage",
  "description": "Report disk usage for root filesystem",
  "command": "df -h / | tail -1 | awk '{print \"{\\\"filesystem\\\": \\\"\" $1 \"\\\", \\\"size\\\": \\\"\" $2 \"\\\", \\\"used\\\": \\\"\" $3 \"\\\", \\\"available\\\": \\\"\" $4 \"\\\", \\\"use_percent\\\": \\\"\" $5 \"\\\"}\"}"
}
```

### Network Connectivity

```json
{
  "name": "ping-check",
  "version": "1.0.0",
  "tool_name": "ping-check",
  "description": "Check network latency to key endpoints",
  "command": "python3 -c \"import json,subprocess; targets=['8.8.8.8','1.1.1.1']; results={}; [results.update({t: float(subprocess.run(['ping','-c','1','-W','2',t],capture_output=True,text=True).stdout.split('time=')[1].split(' ')[0]) if 'time=' in subprocess.run(['ping','-c','1','-W','2',t],capture_output=True,text=True).stdout else -1}) for t in targets]; print(json.dumps(results))\""
}
```

## Data Processing

### Log Tail

```json
{
  "name": "log-tail",
  "version": "1.0.0",
  "tool_name": "log-tail",
  "description": "Return last 20 lines of syslog",
  "command": "python3 -c \"import json; lines=open('/var/log/syslog').readlines()[-20:]; print(json.dumps({'lines':[l.strip() for l in lines],'count':len(lines)}))\""
}
```

### File Watcher

```json
{
  "name": "recent-files",
  "version": "1.0.0",
  "tool_name": "recent-files",
  "description": "List files modified in the last hour",
  "command": "find /home -maxdepth 3 -mmin -60 -type f 2>/dev/null | head -20 | python3 -c \"import json,sys; print(json.dumps({'files': [l.strip() for l in sys.stdin]}))\""
}
```

## macOS Specific

### System Info

```json
{
  "name": "mac-sysinfo",
  "version": "1.0.0",
  "tool_name": "mac-sysinfo",
  "description": "macOS system information: hostname, OS version, chip, memory",
  "command": "python3 -c \"import json,platform,subprocess; sp=subprocess.run(['sysctl','-n','hw.memsize'],capture_output=True,text=True); print(json.dumps({'hostname':platform.node(),'os':platform.mac_ver()[0],'arch':platform.machine(),'chip':platform.processor(),'ram_gb':round(int(sp.stdout.strip())/1e9,1)}))\""
}
```

## EAL Mission Templates

### Device Fleet Health Check

```eal
mission "fleet-health" {
  let cam_health = call "health-check" on "camera-01" with {} timeout 10
  let gpu_health = call "health-check" on "gpu-node" with {} timeout 10
  let nas_health = call "health-check" on "nas-server" with {} timeout 10

  let report = call "summarize" on "claude" with {
    prompt = "Summarize the health status of these 3 devices. Flag any issues.",
    camera = cam_health.output,
    gpu = gpu_health.output,
    nas = nas_health.output
  } timeout 60
}
```

### Agent Code Review Pipeline

```eal
mission "pr-review" {
  let security = call "security-review" on "claude" with {
    prompt = "Review this diff for security vulnerabilities. Be thorough."
  } timeout 120

  let perf = call "perf-review" on "codex" with {
    prompt = "Analyze for performance regressions and optimization opportunities."
  } timeout 120

  let style = call "style-review" on "claude" with {
    prompt = "Check code style, naming, and documentation completeness."
  } timeout 60

  let final_review = call "compile-review" on "claude" with {
    prompt = "Compile a unified PR review from these three analyses. Prioritize by severity.",
    security_findings = security.output,
    perf_findings = perf.output,
    style_findings = style.output
  } timeout 120
}
```

### Self-Improving Ability (Agent generates + deploys)

```eal
mission "auto-ability" {
  // Ask Claude to design an ability for the given task
  let design = call "design-ability" on "claude" with {
    prompt = "Design an ability.json that checks if a Docker container named 'web' is running and healthy. Output ONLY the JSON, no markdown."
  } timeout 60

  // Deploy the designed ability (via MCP deploy tool if available)
  // For now, the output can be manually deployed:
  // easynet deploy <dir> --to <node>
}
```
