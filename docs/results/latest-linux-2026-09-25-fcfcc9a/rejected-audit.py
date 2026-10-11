import collections,csv,hashlib,json,math,pathlib,re,statistics,tarfile
ROOT=pathlib.Path(__file__).resolve().parents[3]; OUT=pathlib.Path(__file__).resolve().parent
sha=lambda p:hashlib.sha256(p.read_bytes()).hexdigest()
rows=list(csv.DictReader((OUT/'current.csv').open()));meta=json.loads((OUT/'current.csv.metadata.json').read_text());summary=json.loads((OUT/'summary.json').read_text())
expected={(f,m,r) for f in ['lumine','gpui','quickgui'] for m in ['idle','stream','scroll','both'] for r in range(3)}
actual=[(x['framework'],x['mode'],int(x['repeat'])) for x in rows]
assert len(rows)==36 and set(actual)==expected
# Verify the exact preregistered order rather than merely inventory.
order=[]; frameworks=['lumine','gpui','quickgui']
for repeat in range(3):
 for mode in ['idle','stream','scroll','both']:
  order.extend((f,mode,repeat) for f in frameworks[repeat:]+frameworks[:repeat])
assert actual==order
sample_count=0;rates=[];wall=[];violations=[];framework_rates=collections.defaultdict(list)
for row in rows:
 stem=OUT/f"current.csv.{row['framework']}.{row['mode']}.{row['repeat']}"
 raw=json.loads(pathlib.Path(str(stem)+'.json').read_text()); log=pathlib.Path(str(stem)+'.log').read_text(); s=raw['samples']
 assert raw['requested_seconds']==20 and raw['warmup_seconds']==5 and int(row['exit_code'])==0
 assert len(s)>=2 and all(x['elapsed_seconds']>=5 and x['rss_bytes']>0 for x in s)
 assert all(b['elapsed_seconds']>a['elapsed_seconds'] and b['cpu_seconds']>=a['cpu_seconds'] for a,b in zip(s,s[1:]))
 duration=s[-1]['elapsed_seconds']-s[0]['elapsed_seconds'];cpu=s[-1]['cpu_seconds']-s[0]['cpu_seconds']
 computed={'wall_seconds':duration,'cpu_seconds':cpu,'cpu_percent_one_core':cpu/duration*100,'mean_rss_bytes':statistics.mean(x['rss_bytes'] for x in s),'peak_rss_bytes':max(x['rss_bytes'] for x in s),'samples':len(s)}
 for field,value in computed.items():
  assert math.isclose(float(row[field]),value,rel_tol=1e-10,abs_tol=1e-7),(stem,field)
  assert math.isclose(float(raw['summary'][field]),value,rel_tol=1e-10,abs_tol=1e-7),(stem,'raw',field)
 match=re.search(r'workload_ticks=(\d+)|"ticks":(\d+)',log);assert match
 ticks=int(next(v for v in match.groups() if v is not None));assert ticks==int(row['workload_ticks'])==raw['summary']['workload_ticks']
 if row['mode']=='idle':assert ticks==0
 else:
  rate=ticks/20;rates.append(rate);framework_rates[row['framework']].append(rate)
  if not 58<=rate<=61:violations.append({'framework':row['framework'],'mode':row['mode'],'repeat':int(row['repeat']),'ticks':ticks,'ticks_per_requested_second':rate})
 if row['framework']=='lumine':
  reports=[v for v in raw['application_reports'] if v.get('framework')=='lumine'];assert len(reports)==1 and reports[0]['adapter']=='components' and reports[0]['renderer']=='gpu'
  assert 0<=reports[0]['mounted_rows']<=19
 sample_count+=len(s);wall.append(duration)
assert len(summary['groups'])==12
seen=set()
for group in summary['groups']:
 key=(group['framework'],group['mode']);assert key not in seen;seen.add(key)
 trials=[r for r in rows if (r['framework'],r['mode'])==key];assert len(trials)==group['repeats']==3
 for field in ['cpu_percent_one_core','mean_rss_bytes','peak_rss_bytes','workload_ticks','wall_seconds']:
  values=[float(r[field]) for r in trials]
  for statistic,value in {'min':min(values),'max':max(values),'median':statistics.median(values)}.items():assert math.isclose(group[field][statistic],value,rel_tol=1e-12,abs_tol=1e-7),(key,field,statistic)
hashes=json.loads((OUT/'source.json').read_text())
with tarfile.open(OUT/'source.tar.gz','r:gz') as archive:
 entries=[x for x in archive.getmembers() if x.isfile()];assert len(entries)==len(hashes)
 assert {x.name for x in entries}==set(hashes)
 for entry in entries:assert hashlib.sha256(archive.extractfile(entry).read()).hexdigest()==hashes[entry.name],entry.name
current_mismatches=[name for name,expected in hashes.items() if sha(ROOT/name)!=expected]
# Do not silently treat post-measurement documentation edits as source identity.
builds=[json.loads((OUT/name).read_text()) for name in ['lumine-build-manifest.json','reference-build-manifest.json']]
build_input_mismatches=[]
for manifest in builds:
 for name,expected in manifest['source_sha256'].items():
  assert hashes[name]==expected,name
  if sha(ROOT/name)!=expected:build_input_mismatches.append(name)
preflight=json.loads((OUT/'preflight.json').read_text());binary_paths={}
for f,filename in {'lumine':'component_workload','gpui':'lumine-compare-gpui','quickgui':'lumine-compare-quickgui'}.items():
 manifest=builds[0] if f=='lumine' else builds[1];path=pathlib.Path('/tmp/lumine-latest-linux-2026-09-25-fcfcc9a-bin')/filename
 assert sha(path)==manifest['binaries'][f]['sha256']==preflight['binary_sha256'][f]==meta['binary_sha256'][f];binary_paths[f]=str(path)
loader=pathlib.Path(preflight['loader_path']);assert sha(loader)==preflight['loader_sha256']
env=json.loads((OUT/'measurement-environment.json').read_text());assert env['LP_NUM_THREADS']=='4' and env['LIBGL_ALWAYS_SOFTWARE']=='1' and env['WGPU_BACKEND']=='vulkan' and env['WINIT_UNIX_BACKEND']=='x11'
observations=[json.loads(x) for x in (OUT/'host-observations.jsonl').read_text().splitlines()];assert len(observations)>100
assert all(b['unix_time']>a['unix_time'] for a,b in zip(observations,observations[1:]))
processes=collections.defaultdict(lambda:{'observations':0,'pids':set(),'max_ps_cpu':0.})
for observation in observations:
 names=set()
 for line in observation['processes'].splitlines()[1:]:
  pid,ppid,name,cpu,memory=line.split();names.add(name);processes[name]['pids'].add(int(pid));processes[name]['max_ps_cpu']=max(processes[name]['max_ps_cpu'],float(cpu))
 for name in names:processes[name]['observations']+=1
build_processes=[name for name in ['cargo','rustc','rust-lld'] if name in processes]
workers={}
for f in frameworks:
 threads=json.loads((OUT/f'capture-{f}-threads.json').read_text());workers[f]={'all_threads':len(threads),'llvmpipe_workers':sum(v['comm'].startswith('llvmpipe-') for v in threads)}
result={'status':'rejected_workload_parity' if violations else ('passed' if not current_mismatches and not build_input_mismatches else 'frozen_artifacts_pass_current_inputs_changed'),'trials':len(rows),'samples':sample_count,'exact_rotated_trial_order':True,'raw_and_csv_recomputed':True,'all_summary_medians_and_ranges_recomputed':True,'ticks_per_requested_second':{'min':min(rates),'max':max(rates)},'sampled_wall_seconds':{'min':min(wall),'max':max(wall)},'archived_inputs':len(hashes),'archive_inventory_and_hashes_match':True,'current_archive_input_mismatches':current_mismatches,'current_build_input_mismatches':sorted(set(build_input_mismatches)),'all_three_binary_hashes_and_loader_verified':True,'binary_paths':binary_paths,'host_observations':len(observations),'host_observation_span_seconds':observations[-1]['unix_time']-observations[0]['unix_time'],'observed_build_process_names':build_processes,'host_context':{name:{**processes[name],'pids':len(processes[name]['pids'])} for name in ['zeron','git','gh'] if name in processes},'capture_thread_counts':workers,'limitations':['Ticks count logical updates, not presented frames.','Shared host activity is recorded in host observations; ps CPU is lifetime-averaged, not interval contention.','LP_NUM_THREADS=4 applies to each Mesa worker pool; QuickGUI capture shows two pools.','Software Vulkan on Xvfb does not establish native macOS, hardware-GPU or universal performance rankings.','A measured 0% CPU means no accounted CPU increment between endpoint samples, not proof of literally zero cycles.']}
result.update({'parity_violations':violations,'near_60hz_workload_updates':not violations,'framework_updates_per_requested_second':{f:{'min':min(v),'max':max(v)} for f,v in framework_rates.items()},'protocol_deviation':json.loads((OUT/'protocol-deviation.json').read_text()),'active_builds_at_start':True,'integrity_checks_passed':not current_mismatches and not build_input_mismatches})
(OUT/'independent-audit.json').write_text(json.dumps(result,indent=2)+'\n');print(json.dumps(result,indent=2))
