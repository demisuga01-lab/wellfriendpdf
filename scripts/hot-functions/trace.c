// LD_PRELOAD tracer: record the first instruction executed in each page of the
// executable's text, re-arming every TRACE_EPOCH_US microseconds so later
// first-touches are seen too. Output: TRACE_OUT file, one "offset" (hex, file
// offset of the faulting address) per line.
#define _GNU_SOURCE
#include <signal.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/mman.h>
#include <sys/time.h>
#include <unistd.h>

static unsigned long text_lo, text_hi, text_off;
static unsigned long *hits;
static volatile unsigned long nhits;
static unsigned long cap = 1 << 22;

static void on_segv(int sig, siginfo_t *si, void *ctx) {
  unsigned long a = (unsigned long)si->si_addr;
  if (a < text_lo || a >= text_hi) {
    signal(SIGSEGV, SIG_DFL);
    return;
  }
  unsigned long n = __atomic_fetch_add(&nhits, 1, __ATOMIC_RELAXED);
  if (n < cap) hits[n] = a - text_lo + text_off;
  mprotect((void *)(a & ~4095UL), 4096, PROT_READ | PROT_EXEC);
}

static void on_alarm(int sig) {
  mprotect((void *)text_lo, text_hi - text_lo, PROT_NONE);
}

__attribute__((constructor)) static void init(void) {
  if (!getenv("TRACE_OUT")) return;
  { char e[512]; ssize_t m = readlink("/proc/self/exe", e, 511); if (m <= 0) return; e[m] = 0; if (getenv("TRACE_EXE") && !strstr(e, getenv("TRACE_EXE"))) return; }
  char exe[512];
  ssize_t n = readlink("/proc/self/exe", exe, 511);
  exe[n] = 0;
  FILE *in = fopen("/proc/self/maps", "r");
  char l[1024];
  while (fgets(l, sizeof l, in)) {
    unsigned long a, b, off;
    char perm[8];
    if (sscanf(l, "%lx-%lx %7s %lx", &a, &b, perm, &off) == 4 && strstr(l, exe) && perm[2] == 'x') {
      text_lo = a; text_hi = b; text_off = off;
    }
  }
  fclose(in);
  hits = mmap(0, cap * sizeof(unsigned long), PROT_READ | PROT_WRITE, MAP_PRIVATE | MAP_ANONYMOUS, -1, 0);
  struct sigaction sa;
  memset(&sa, 0, sizeof sa);
  sa.sa_sigaction = on_segv;
  sa.sa_flags = SA_SIGINFO | SA_RESTART | SA_NODEFER;
  sigaction(SIGSEGV, &sa, 0);
  struct sigaction sp;
  memset(&sp, 0, sizeof sp);
  sp.sa_handler = on_alarm;
  sp.sa_flags = SA_RESTART;
  sigaction(SIGPROF, &sp, 0);
  long us = getenv("TRACE_EPOCH_US") ? atol(getenv("TRACE_EPOCH_US")) : 2000;
  struct itimerval it = {{0, us}, {0, us}};
  setitimer(ITIMER_PROF, &it, 0);
  mprotect((void *)text_lo, text_hi - text_lo, PROT_NONE);
}

__attribute__((destructor)) static void fini(void) {
  if (!hits) return;
  struct itimerval it = {{0, 0}, {0, 0}};
  setitimer(ITIMER_PROF, &it, 0);
  mprotect((void *)text_lo, text_hi - text_lo, PROT_READ | PROT_EXEC);
  FILE *out = fopen(getenv("TRACE_OUT"), "a");
  unsigned long n = nhits < cap ? nhits : cap;
  for (unsigned long i = 0; i < n; i++) fprintf(out, "%lx\n", hits[i]);
  fclose(out);
}
