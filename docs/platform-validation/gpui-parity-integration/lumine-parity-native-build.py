import subprocess,os,json,pathlib,time
root=pathlib.Path('/home/ubuntu/GitHub/lumine');out=pathlib.Path('/tmp/lumine-parity-integrated')
env=dict(os.environ,CARGO_TARGET_DIR='/tmp/lumine-target',CARGO_PROFILE_DEV_DEBUG='0',CARGO_INCREMENTAL='0',CARGO_BUILD_JOBS='1',LD_LIBRARY_PATH='/tmp/lumine-ci-loader-check/build/loader',LP_NUM_THREADS='4')
cmd=['cargo','build','-p','lumine-desktop','--locked']
for name in ['paint_styles','canvas','svg_transform','animated_image','cursors','rich_text','actions','layout','drag_drop','dynamic_menus','platform_services','native_platform','component_workload','effects']:
 cmd+=['--example',name]
with (out/'native-build.log').open('w') as log:r=subprocess.run(cmd,cwd=root,env=env,stdout=log,stderr=subprocess.STDOUT)
(out/'native-build.json').write_text(json.dumps({'command':cmd,'exit_code':r.returncode},indent=2)+'\n')
raise SystemExit(r.returncode)
