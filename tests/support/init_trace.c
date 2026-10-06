// Linux/x86_64 public-process fault fixture. No production fault API.
// Trace only writes to this fixture's Work paths; the parent owns the child.
#define _GNU_SOURCE
#include <sys/ptrace.h>
#include <sys/wait.h>
#include <sys/user.h>
#include <sys/syscall.h>
#include <signal.h>
#include <fcntl.h>
#include <unistd.h>
#include <errno.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <limits.h>
static void die(const char *what) { perror(what); exit(2); }
static void memory(pid_t pid,unsigned long address,char *out,size_t capacity) {
  size_t at=0; while(at+sizeof(long)<capacity) { errno=0;long word=ptrace(PTRACE_PEEKDATA,pid,address+at,0);if(errno)die("peek");memcpy(out+at,&word,sizeof word);if(memchr(&word,0,sizeof word))return;at+=sizeof word; }out[capacity-1]=0;
}
static void fdpath(pid_t pid,long fd,char *out) {
  char proc[128];snprintf(proc,sizeof proc,"/proc/%d/fd/%ld",pid,fd);ssize_t n=readlink(proc,out,PATH_MAX-1);if(n<0){out[0]=0;return;}out[n]=0;
}
static void target(pid_t pid,long fd,unsigned long address,char *out) {
  char relative[PATH_MAX],base[PATH_MAX];memory(pid,address,relative,sizeof relative);
  if(relative[0]=='/'){snprintf(out,PATH_MAX,"%s",relative);return;}
  fdpath(pid,fd,base);if(!base[0]){out[0]=0;return;}
  if(snprintf(out,PATH_MAX,"%s/%s",base,relative)>=PATH_MAX)die("path length");
}
int main(int argc,char **argv) {
  if(argc<5){fprintf(stderr,"usage: tracer index kill|fail|log|pause log-file executable [args]\n");return 2;}
  long stop=strtol(argv[1],0,10),count=0;FILE *log=fopen(argv[3],"w");if(!log)die("log");
  pid_t pid=fork();if(pid<0)die("fork");if(pid==0){if(ptrace(PTRACE_TRACEME,0,0,0)<0)die("traceme");raise(SIGSTOP);execv(argv[4],argv+4);die("exec");}
  int status;if(waitpid(pid,&status,0)<0)die("wait");if(ptrace(PTRACE_SETOPTIONS,pid,0,PTRACE_O_TRACESYSGOOD|PTRACE_O_EXITKILL)<0)die("options");
  int entering=1,mutation=0,inject=0;long syscall=0;char file[PATH_MAX]={0};
  for(;;){if(ptrace(PTRACE_SYSCALL,pid,0,0)<0)die("syscall");if(waitpid(pid,&status,0)<0)die("wait");
    if(WIFEXITED(status)){fclose(log);return WEXITSTATUS(status);}if(WIFSIGNALED(status)){fclose(log);return 128+WTERMSIG(status);}
    if(WSTOPSIG(status)!=(SIGTRAP|0x80))continue;
    struct user_regs_struct r;if(ptrace(PTRACE_GETREGS,pid,0,&r)<0)die("regs");
    if(entering){syscall=(long)r.orig_rax;file[0]=0;mutation=0;inject=0;
      if(syscall==SYS_mkdirat||syscall==SYS_unlinkat){target(pid,r.rdi,r.rsi,file);mutation=1;}
      if(syscall==SYS_renameat2){target(pid,r.rdx,r.r10,file);mutation=1;}
      if(syscall==SYS_openat&&(r.rdx&O_CREAT)){target(pid,r.rdi,r.rsi,file);mutation=1;}
      if(syscall==SYS_write||syscall==SYS_fsync||syscall==SYS_fchmod){fdpath(pid,r.rdi,file);mutation=1;}
      mutation=mutation&&(strstr(file,"/.work")||strstr(file,"/work-beta2")||strstr(file,"/.gitignore")||strstr(file,"/.stage-"));
      if(mutation){++count;fprintf(log,"%ld %ld %s\n",count,syscall,file);fflush(log);
        if(count==stop&&!strcmp(argv[2],"fail")){r.orig_rax=(unsigned long)-1;if(ptrace(PTRACE_SETREGS,pid,0,&r)<0)die("inject");inject=1;}
      }
    }else{
      if(inject){r.rax=(unsigned long)-EIO;if(ptrace(PTRACE_SETREGS,pid,0,&r)<0)die("error");}
      if(mutation&&count==stop&&!strcmp(argv[2],"pause")){
        char ready[PATH_MAX],resume[PATH_MAX];if(snprintf(ready,sizeof ready,"%s.ready",argv[3])>=PATH_MAX||snprintf(resume,sizeof resume,"%s.resume",argv[3])>=PATH_MAX)die("pause path");
        int fd=open(ready,O_WRONLY|O_CREAT|O_EXCL,0600);if(fd<0)die("pause ready");if(close(fd)<0)die("pause close");
        int waited=0;while(access(resume,F_OK)<0){if(errno!=ENOENT)die("pause poll");if(waited++>=6000){kill(pid,SIGKILL);waitpid(pid,&status,0);fclose(log);return 2;}usleep(10000);}
      }
      if(mutation&&count==stop&&!strcmp(argv[2],"kill")){if(kill(pid,SIGKILL)<0)die("kill");if(waitpid(pid,&status,0)<0)die("reap");fclose(log);return 137;}
    }
    entering=!entering;
  }
}
