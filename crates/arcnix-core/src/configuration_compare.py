"""One bounded file snapshot. No writes, shell execution, or directory listing."""
import base64,hashlib,json,os,pathlib,signal,stat,sys,time
LIMIT=128*1024
class Refused(Exception):pass
def need(ok,issue):
    if not ok:raise Refused(issue)
def token(info):return (info.st_dev,info.st_ino,info.st_mode,info.st_uid,info.st_gid,info.st_nlink,info.st_size,info.st_mtime_ns,info.st_ctime_ns)
def allowed(path):
    return path.startswith('/') and path!='/' and len(path.encode())<=1024 and not any(ord(c)<32 or 127<=ord(c)<160 for c in path) and not any(p in ['.','..'] for p in path.split('/')) and not any(path==p or path.startswith(p+'/') for p in ['/proc','/sys','/dev'])
def snapshot(path):
    need(sys.platform.startswith('linux') and hasattr(os,'O_PATH'),'unsupported_path')
    need(allowed(path),'unsupported_path')
    original=os.lstat(path);resolved=str(pathlib.Path(path).resolve(strict=True));need(allowed(resolved),'unsupported_path')
    parent=os.open('/',os.O_RDONLY|os.O_DIRECTORY|os.O_CLOEXEC)
    source=None;pinned=None
    try:
        parts=pathlib.PurePosixPath(resolved).parts[1:]
        for name in parts[:-1]:
            next_fd=os.open(name,os.O_RDONLY|os.O_DIRECTORY|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=parent);os.close(parent);parent=next_fd
        pinned=os.open(parts[-1],os.O_PATH|os.O_NOFOLLOW|os.O_CLOEXEC,dir_fd=parent)
        need(stat.S_ISREG(os.fstat(pinned).st_mode),'not_regular')
        source=os.open('/proc/self/fd/'+str(pinned),os.O_RDONLY|os.O_NONBLOCK|os.O_CLOEXEC)
        before=os.fstat(source);need(stat.S_ISREG(before.st_mode),'not_regular');need(before.st_size<=LIMIT,'too_large')
        data=bytearray()
        while True:
            block=os.read(source,min(16384,LIMIT+1-len(data)))
            if not block:break
            data.extend(block);need(len(data)<=LIMIT,'too_large')
        need(token(before)==token(os.fstat(source)) and token(before)==token(os.stat(resolved,follow_symlinks=False)) and token(original)==token(os.lstat(path)) and str(pathlib.Path(path).resolve(strict=True))==resolved,'changed')
        try:content=data.decode('utf-8')
        except UnicodeDecodeError:raise Refused('binary')
        need('\0' not in content,'binary')
        return dict(canonical=resolved,bytes=len(data),sha256=hashlib.sha256(data).hexdigest(),content=base64.b64encode(data).decode(),uid=before.st_uid,gid=before.st_gid,mode=stat.S_IMODE(before.st_mode),symlink=path!=resolved)
    finally:
        if source is not None:os.close(source)
        if pinned is not None:os.close(pinned)
        os.close(parent)
def main():
    signal.alarm(12);request=json.loads(base64.b64decode(sys.argv[1],validate=True));need(request.get('version')==1 and set(request)=={'version','path'},'invalid_request');path=request['path'];need(isinstance(path,str),'invalid_request')
    result=dict(version=1,path=path,sampled=int(time.time()),snapshot=None,issue=None)
    try:result['snapshot']=snapshot(path)
    except Refused as error:result['issue']=str(error)
    except FileNotFoundError:result['issue']='missing'
    except PermissionError:result['issue']='denied'
    except (OSError,RuntimeError,ValueError):result['issue']='unavailable'
    print(json.dumps(result,ensure_ascii=True,separators=(',',':')));signal.alarm(0)
if __name__=='__main__':main()
