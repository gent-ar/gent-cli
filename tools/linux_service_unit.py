"""Render an opt-in user service; never installs, enables, or starts it.
Requires systemd >=254 DelegateSubgroup and a cgroup2 nsdelegate mount.
"""
from pathlib import Path

def argument(value):
    value=str(value)
    if not value or any(ord(c)<32 or ord(c)==127 for c in value):
        raise ValueError("service arguments must contain no control characters")
    return '"'+value.replace("\\","\\\\").replace('"','\\"').replace("%","%%").replace("$","$$")+'"'

def render(bundle, data, systemd_version):
    if systemd_version < 254:
        raise ValueError("systemd >=254 is required for DelegateSubgroup=controller")
    bundle=Path(bundle);data=Path(data)
    for path in (bundle,data):
        if not path.is_absolute() or path.resolve(strict=True)!=path or not path.is_dir() or any(p.is_symlink() for p in (path,*path.parents)):
            raise ValueError("canonical existing release/data directories required")
    exe=bundle/"gentd"
    if exe.is_symlink() or not exe.is_file(): raise ValueError("exact packaged gentd required")
    command=' '.join(argument(x) for x in (exe,"--standalone-authority","--prepare-linux-service-delegation","--data-dir",data))
    return ("# Generated opt-in service; source preparation is not installation.\n"
            "[Unit]\nDescription=Gent runtime host with owned Linux delegation\n\n"
            "[Service]\nType=simple\nDelegate=memory pids\nDelegateSubgroup=controller\n"
            "KillMode=control-group\nExecStart="+command+"\n\n[Install]\nWantedBy=default.target\n")

def render_installed(version,systemd_version):
    import re
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+",version): raise ValueError("finite numeric release version required")
    if systemd_version<254: raise ValueError("systemd>=254 required")
    exe="/usr/lib/gent/"+version+"/gentd"
    command=' '.join(argument(x)for x in (exe,"--standalone-authority","--prepare-linux-service-delegation"))
    # Default data directory is resolved by Gent itself; never duplicate that policy here.
    return "[Unit]\nDescription=Gent contained runtime (explicit opt-in)\n\n[Service]\nType=simple\nDelegate=memory pids\nDelegateSubgroup=controller\nKillMode=control-group\nExecStart="+command+"\n\n[Install]\nWantedBy=default.target\n"
