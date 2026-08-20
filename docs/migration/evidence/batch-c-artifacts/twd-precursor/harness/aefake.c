/* aefake — controllable fake agent for the T-WD producer precursor.
 * Non-shell foreground identity. Reads stdin without echoing (logs it).
 * Prints controller-driven lines received on a control FIFO.
 * Env: AEFAKE_LOG (append raw stdin + argv), AEFAKE_CTL (control fifo path),
 *      AEFAKE_BANNER (fixed startup line).
 */
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#include <fcntl.h>
#include <poll.h>
#include <signal.h>
#include <termios.h>

static struct termios orig_t;
static int have_t = 0;

static void restore(void) {
    if (have_t) tcsetattr(0, TCSANOW, &orig_t);
}
static void onsig(int s) { (void)s; restore(); _exit(0); }

int main(int argc, char **argv) {
    const char *logp = getenv("AEFAKE_LOG");
    const char *ctlp = getenv("AEFAKE_CTL");
    const char *banner = getenv("AEFAKE_BANNER");
    FILE *lg = NULL;
    int i;

    if (logp && *logp) lg = fopen(logp, "a");
    if (lg) {
        fprintf(lg, "--- aefake start pid=%d ---\n", (int)getpid());
        for (i = 0; i < argc; i++) fprintf(lg, "argv[%d]=%s\n", i, argv[i]);
        fflush(lg);
    }

    signal(SIGTERM, onsig);
    signal(SIGINT, onsig);
    signal(SIGHUP, onsig);

    if (isatty(0) && tcgetattr(0, &orig_t) == 0) {
        struct termios t = orig_t;
        have_t = 1;
        t.c_lflag &= ~(ECHO | ECHOE | ECHOK | ECHONL | ICANON);
        t.c_cc[VMIN] = 1;
        t.c_cc[VTIME] = 0;
        tcsetattr(0, TCSANOW, &t);
    }

    printf("%s\n", (banner && *banner) ? banner : "aefake ready");
    fflush(stdout);

    int ctl = -1;
    if (ctlp && *ctlp) ctl = open(ctlp, O_RDWR | O_NONBLOCK);

    struct pollfd fds[2];
    char buf[8192];
    for (;;) {
        int nf = 0;
        fds[nf].fd = 0; fds[nf].events = POLLIN; nf++;
        if (ctl >= 0) { fds[nf].fd = ctl; fds[nf].events = POLLIN; nf++; }
        int r = poll(fds, nf, 1000);
        if (r < 0) continue;
        if (fds[0].revents & POLLIN) {
            ssize_t n = read(0, buf, sizeof buf);
            if (n > 0) {
                if (lg) { fwrite(buf, 1, (size_t)n, lg); fflush(lg); }
            } else if (n == 0) {
                /* stdin closed: keep running, stop polling it */
                fds[0].events = 0;
            }
        }
        if (ctl >= 0 && nf > 1 && (fds[1].revents & POLLIN)) {
            ssize_t n = read(ctl, buf, sizeof buf - 1);
            if (n > 0) {
                buf[n] = 0;
                if (strncmp(buf, "__EXIT__", 8) == 0) { restore(); return 0; }
                fwrite(buf, 1, (size_t)n, stdout);
                fflush(stdout);
            }
        }
    }
}
