#if defined(__APPLE__)
#define _DARWIN_C_SOURCE 1
#endif

#define _GNU_SOURCE
#define _XOPEN_SOURCE 700
#define _POSIX_C_SOURCE 200809L

#include <ctype.h>
#include <errno.h>
#include <fcntl.h>
#include <limits.h>
#include <pwd.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <sys/types.h>
#include <unistd.h>

#ifndef PATH_MAX
#define PATH_MAX 4096
#endif

struct strlist {
    char **items;
    size_t len;
    size_t cap;
};

struct strbuf {
    char *data;
    size_t len;
    size_t cap;
};

static void __attribute__((noreturn, format(printf, 1, 2))) dief(const char *fmt, ...) {
    va_list ap;
    fprintf(stderr, "sbbash: ");
    va_start(ap, fmt);
    vfprintf(stderr, fmt, ap);
    va_end(ap);
    fputc('\n', stderr);
    exit(111);
}

static void __attribute__((noreturn)) die_errno(const char *what) {
    dief("%s: %s", what, strerror(errno));
}

static void *xmalloc(size_t n) {
    void *out = malloc(n);
    if (!out) {
        dief("out of memory");
    }
    return out;
}

static void *xrealloc(void *ptr, size_t n) {
    void *out = realloc(ptr, n);
    if (!out) {
        dief("out of memory");
    }
    return out;
}

static char *xstrdup(const char *s) {
    size_t n = strlen(s) + 1;
    char *out = xmalloc(n);
    memcpy(out, s, n);
    return out;
}

static char *path_join(const char *a, const char *b) {
    size_t alen = strlen(a);
    size_t blen = strlen(b);
    bool need_slash = (alen > 0 && a[alen - 1] != '/');
    size_t n = alen + (need_slash ? 1 : 0) + blen + 1;
    char *out = xmalloc(n);
    memcpy(out, a, alen);
    size_t pos = alen;
    if (need_slash) {
        out[pos++] = '/';
    }
    memcpy(out + pos, b, blen);
    out[pos + blen] = '\0';
    return out;
}

static void strlist_append_owned(struct strlist *list, char *item) {
    if (list->len == list->cap) {
        size_t new_cap = (list->cap == 0) ? 4 : list->cap * 2;
        list->items = xrealloc(list->items, new_cap * sizeof(char *));
        list->cap = new_cap;
    }
    list->items[list->len++] = item;
}

static bool strlist_contains(const struct strlist *list, const char *item) {
    for (size_t i = 0; i < list->len; ++i) {
        if (strcmp(list->items[i], item) == 0) {
            return true;
        }
    }
    return false;
}

static void strlist_append_unique_owned(struct strlist *list, char *item) {
    if (strlist_contains(list, item)) {
        free(item);
        return;
    }
    strlist_append_owned(list, item);
}

static void strlist_append_dup(struct strlist *list, const char *item) {
    strlist_append_owned(list, xstrdup(item));
}

static void strbuf_reserve(struct strbuf *buf, size_t extra) {
    size_t need = buf->len + extra + 1;
    if (need <= buf->cap) {
        return;
    }
    size_t new_cap = buf->cap ? buf->cap : 128;
    while (new_cap < need) {
        new_cap *= 2;
    }
    buf->data = xrealloc(buf->data, new_cap);
    buf->cap = new_cap;
}

static void strbuf_append_len(struct strbuf *buf, const char *s, size_t n) {
    strbuf_reserve(buf, n);
    memcpy(buf->data + buf->len, s, n);
    buf->len += n;
    buf->data[buf->len] = '\0';
}

static void strbuf_append(struct strbuf *buf, const char *s) {
    strbuf_append_len(buf, s, strlen(s));
}

static void __attribute__((format(printf, 2, 3))) strbuf_appendf(struct strbuf *buf, const char *fmt, ...) {
    va_list ap;
    va_list copy;
    va_start(ap, fmt);
    va_copy(copy, ap);
    int needed = vsnprintf(NULL, 0, fmt, copy);
    va_end(copy);
    if (needed < 0) {
        va_end(ap);
        dief("vsnprintf failed");
    }
    strbuf_reserve(buf, (size_t)needed);
    vsnprintf(buf->data + buf->len, buf->cap - buf->len, fmt, ap);
    va_end(ap);
    buf->len += (size_t)needed;
}

static char *trim_whitespace(char *s) {
    while (*s && isspace((unsigned char)*s)) {
        ++s;
    }
    size_t n = strlen(s);
    while (n > 0 && isspace((unsigned char)s[n - 1])) {
        s[--n] = '\0';
    }
    return s;
}

static char *expand_home_path(const char *path, const char *host_home) {
    if (path[0] != '~') {
        return xstrdup(path);
    }
    if (!host_home || host_home[0] == '\0') {
        dief("cannot expand %s without a home directory", path);
    }
    if (path[1] == '\0') {
        return xstrdup(host_home);
    }
    if (path[1] == '/') {
        return path_join(host_home, path + 2);
    }
    dief("unsupported home expansion in path %s", path);
}

static const char *base_name(const char *path) {
    const char *slash = strrchr(path, '/');
    return slash ? slash + 1 : path;
}

static void print_help(FILE *out, const char *prog) {
    fprintf(out,
            "Usage: %s [options] [command [args...]]\n"
            "\n"
            "Run commands under macOS sandbox-exec with writes confined to the\n"
            "current directory tree plus any configured extra writable directories.\n"
            "\n"
            "Options:\n"
            "  -h, --help           Show this help and exit\n"
            "  -w, --writable DIR   Allow writes under DIR; may be repeated\n"
            "  --                   Stop parsing %s options and force command mode\n"
            "\n"
            "Behavior:\n"
            "  With no command, start $SHELL interactively.\n"
            "  If the first non-option argument starts with '-', pass arguments to $SHELL.\n"
            "  Otherwise run the given command directly.\n"
            "\n"
            "Config:\n"
            "  $XDG_CONFIG_HOME/sbbash/config or ~/.config/sbbash/config\n"
            "  Format: writable_dir=/absolute/or/~/path\n",
            prog,
            prog);
}

static bool cli_requests_help(int argc, char **argv) {
    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "-h") == 0 || strcmp(argv[i], "--help") == 0) {
            return true;
        }
        if (strcmp(argv[i], "--") == 0) {
            return false;
        }
        if (strcmp(argv[i], "-w") == 0 || strcmp(argv[i], "--writable") == 0) {
            if (i + 1 < argc) {
                ++i;
            }
            continue;
        }
        if (strncmp(argv[i], "--writable=", 11) == 0) {
            continue;
        }
        break;
    }
    return false;
}

static bool shell_is_bash(const char *shell_path) {
    return strcmp(base_name(shell_path), "bash") == 0;
}

static const char *history_file_name_for_shell(const char *shell_path) {
    const char *name = base_name(shell_path);
    if (strcmp(name, "bash") == 0) {
        return ".bash_history";
    }
    if (strcmp(name, "zsh") == 0) {
        return ".zsh_history";
    }
    return ".sh_history";
}

static const char *pick_shell(const char *env_shell, const char *pw_shell) {
    if (env_shell && env_shell[0] == '/' && access(env_shell, X_OK) == 0) {
        return env_shell;
    }
    if (pw_shell && pw_shell[0] == '/' && access(pw_shell, X_OK) == 0) {
        return pw_shell;
    }
    if (access("/bin/bash", X_OK) == 0) {
        return "/bin/bash";
    }
    dief("could not find an executable shell from $SHELL, passwd entry, or /bin/bash");
}

static char *config_path(const char *host_home, const char *xdg_config_home) {
    char *config_root = NULL;
    if (xdg_config_home && xdg_config_home[0] == '/') {
        config_root = xstrdup(xdg_config_home);
    } else if (host_home && host_home[0] != '\0') {
        config_root = path_join(host_home, ".config");
    } else {
        return NULL;
    }

    char *config_dir = path_join(config_root, "sbbash");
    free(config_root);
    char *path = path_join(config_dir, "config");
    free(config_dir);
    return path;
}

static void load_config_writable_dirs(struct strlist *dirs, const char *host_home, const char *xdg_config_home) {
    char *path = config_path(host_home, xdg_config_home);
    if (!path) {
        return;
    }

    FILE *fp = fopen(path, "r");
    if (!fp) {
        if (errno == ENOENT) {
            free(path);
            return;
        }
        dief("%s: %s", path, strerror(errno));
    }

    char *line = NULL;
    size_t cap = 0;
    size_t lineno = 0;
    while (getline(&line, &cap, fp) != -1) {
        ++lineno;
        char *s = trim_whitespace(line);
        if (s[0] == '\0' || s[0] == '#') {
            continue;
        }

        char *eq = strchr(s, '=');
        if (!eq) {
            dief("%s:%zu: expected key=value", path, lineno);
        }
        *eq = '\0';
        char *key = trim_whitespace(s);
        char *value = trim_whitespace(eq + 1);
        if (strcmp(key, "writable_dir") != 0) {
            dief("%s:%zu: unknown key %s", path, lineno, key);
        }
        if (value[0] == '\0') {
            dief("%s:%zu: writable_dir requires a path", path, lineno);
        }
        strlist_append_dup(dirs, value);
    }
    if (ferror(fp)) {
        dief("%s: %s", path, strerror(errno));
    }

    free(line);
    fclose(fp);
    free(path);
}

static void parse_cli_options(int argc, char **argv, struct strlist *dirs, int *arg_index, bool *force_command) {
    int i = 1;
    *force_command = false;

    while (i < argc) {
        if (strcmp(argv[i], "--") == 0) {
            *force_command = true;
            ++i;
            break;
        }
        if (strcmp(argv[i], "-w") == 0 || strcmp(argv[i], "--writable") == 0) {
            if (i + 1 >= argc) {
                dief("%s requires a directory argument", argv[i]);
            }
            strlist_append_dup(dirs, argv[i + 1]);
            i += 2;
            continue;
        }
        if (strncmp(argv[i], "--writable=", 11) == 0) {
            const char *value = argv[i] + 11;
            if (value[0] == '\0') {
                dief("--writable requires a directory argument");
            }
            strlist_append_dup(dirs, value);
            ++i;
            continue;
        }
        break;
    }

    *arg_index = i;
}

static char *resolve_writable_dir(const char *path, const char *host_home) {
    char *expanded = expand_home_path(path, host_home);
    char resolved[PATH_MAX];
    if (!realpath(expanded, resolved)) {
        dief("additional writable directory %s: %s", path, strerror(errno));
    }
    free(expanded);

    struct stat st;
    if (stat(resolved, &st) != 0) {
        dief("additional writable directory %s: %s", path, strerror(errno));
    }
    if (!S_ISDIR(st.st_mode)) {
        dief("additional writable directory %s resolves to %s, which is not a directory", path, resolved);
    }
    return xstrdup(resolved);
}

static void resolve_writable_dirs(struct strlist *resolved_dirs, const struct strlist *raw_dirs, const char *host_home) {
    for (size_t i = 0; i < raw_dirs->len; ++i) {
        strlist_append_unique_owned(resolved_dirs, resolve_writable_dir(raw_dirs->items[i], host_home));
    }
}

static bool try_ensure_dir(const char *path, mode_t mode) {
    struct stat st;
    if (lstat(path, &st) == 0) {
        if (S_ISLNK(st.st_mode)) {
            dief("%s exists but is a symlink", path);
        }
        if (!S_ISDIR(st.st_mode)) {
            dief("%s exists but is not a directory", path);
        }
        return true;
    }
    if (errno != ENOENT) {
        if (errno == EACCES || errno == EPERM) {
            return false;
        }
        die_errno("lstat");
    }
    if (mkdir(path, mode) == 0) {
        return true;
    }
    if (errno == EEXIST) {
        if (lstat(path, &st) == 0) {
            if (S_ISLNK(st.st_mode)) {
                dief("%s exists but is a symlink", path);
            }
            if (S_ISDIR(st.st_mode)) {
                return true;
            }
            dief("%s exists but is not a directory", path);
        }
    }
    if (errno == EACCES || errno == EPERM) {
        return false;
    }
    die_errno("mkdir");
    return false;
}

static void close_extra_fds(void) {
    struct rlimit rl;
    if (getrlimit(RLIMIT_NOFILE, &rl) != 0) {
        return;
    }

    rlim_t maxfd = rl.rlim_cur;
    if (maxfd == RLIM_INFINITY || maxfd > 65536) {
        maxfd = 65536;
    }

    for (int fd = 3; fd < (int)maxfd; ++fd) {
        close(fd);
    }
}

#if defined(F_GETPATH)
static bool path_is_within(const char *path, const char *dir) {
    size_t dlen = strlen(dir);
    if (strncmp(path, dir, dlen) != 0) {
        return false;
    }
    return path[dlen] == '\0' || path[dlen] == '/';
}

static bool path_is_within_allowed_dirs(const char *path, const char *workdir, const struct strlist *extra_writable_dirs) {
    if (path_is_within(path, workdir)) {
        return true;
    }
    for (size_t i = 0; i < extra_writable_dirs->len; ++i) {
        if (path_is_within(path, extra_writable_dirs->items[i])) {
            return true;
        }
    }
    return false;
}
#endif

static void refuse_redirected_regular_stdio(const char *workdir, const struct strlist *extra_writable_dirs) {
#if defined(F_GETPATH)
    const char *override = getenv("SBBASH_ALLOW_STDIO_REDIRECTS");
    if (override && override[0] == '1' && override[1] == '\0') {
        return;
    }

    for (int fd = 1; fd <= 2; ++fd) {
        struct stat st;
        if (fstat(fd, &st) != 0) {
            continue;
        }
        if (!S_ISREG(st.st_mode)) {
            continue;
        }

        char raw[PATH_MAX];
        memset(raw, 0, sizeof(raw));
        if (fcntl(fd, F_GETPATH, raw) == -1 || raw[0] == '\0') {
            dief("fd %d is redirected to a regular file outside the sandbox check path; refusing to start", fd);
        }

        char resolved[PATH_MAX];
        const char *final_path = raw;
        if (realpath(raw, resolved) != NULL) {
            final_path = resolved;
        }

        if (!path_is_within_allowed_dirs(final_path, workdir, extra_writable_dirs)) {
            dief("fd %d is redirected to %s outside allowed writable directories; refusing to start (set SBBASH_ALLOW_STDIO_REDIRECTS=1 to override)",
                 fd,
                 final_path);
        }
    }
#else
    (void)workdir;
    (void)extra_writable_dirs;
#endif
}

static void sanitize_env(const char *workdir,
                         const char *sandbox_home,
                         const char *sandbox_tmp,
                         const char *histfile,
                         const char *host_home,
                         const char *user_name,
                         const char *shell_path) {
    const char *term = getenv("TERM");
    const char *lang = getenv("LANG");
    const char *lc_all = getenv("LC_ALL");
    const char *lc_ctype = getenv("LC_CTYPE");
    const char *path = getenv("PATH");

    unsetenv("BASH_ENV");
    unsetenv("ENV");
    unsetenv("DYLD_INSERT_LIBRARIES");
    unsetenv("DYLD_LIBRARY_PATH");
    unsetenv("DYLD_FRAMEWORK_PATH");
    unsetenv("LD_LIBRARY_PATH");

    if (!path || path[0] == '\0') {
        path = "/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin";
    }

    setenv("PATH", path, 1);
    setenv("PWD", workdir, 1);
    setenv("HOME", sandbox_home, 1);
    setenv("TMPDIR", sandbox_tmp, 1);
    setenv("HISTFILE", histfile, 1);
    setenv("SHELL", shell_path, 1);
    setenv("SBBASH_WORKDIR", workdir, 1);
    if (shell_is_bash(shell_path)) {
        setenv("BASH_SILENCE_DEPRECATION_WARNING", "1", 1);
    } else {
        unsetenv("BASH_SILENCE_DEPRECATION_WARNING");
    }

    if (host_home && host_home[0] != '\0') {
        setenv("SBBASH_HOST_HOME", host_home, 1);
    }
    if (user_name && user_name[0] != '\0') {
        setenv("USER", user_name, 1);
        setenv("LOGNAME", user_name, 1);
    }
    if (term && term[0] != '\0') {
        setenv("TERM", term, 1);
    }
    if (lang && lang[0] != '\0') {
        setenv("LANG", lang, 1);
    }
    if (lc_all && lc_all[0] != '\0') {
        setenv("LC_ALL", lc_all, 1);
    }
    if (lc_ctype && lc_ctype[0] != '\0') {
        setenv("LC_CTYPE", lc_ctype, 1);
    }
}

static char *escape_sandbox_string(const char *s) {
    size_t len = strlen(s);
    char *out = xmalloc(len * 2 + 1);
    size_t j = 0;
    for (size_t i = 0; i < len; ++i) {
        if (s[i] == '\n' || s[i] == '\r') {
            dief("path contains a newline: %s", s);
        }
        if (s[i] == '\\' || s[i] == '"') {
            out[j++] = '\\';
        }
        out[j++] = s[i];
    }
    out[j] = '\0';
    return out;
}

static void append_profile_subpath(struct strbuf *buf, const char *path) {
    char *escaped = escape_sandbox_string(path);
    strbuf_appendf(buf, "    (subpath \"%s\")\n", escaped);
    free(escaped);
}

static char *build_profile_text(const struct strlist *extra_writable_dirs) {
    struct strbuf buf = {0};
    strbuf_append(&buf,
                  "(version 1)\n"
                  "(deny default)\n"
                  "(import \"system.sb\")\n"
                  "\n"
                  "; behave like a normal shell, but only allow writes inside WORKDIR\n"
                  "(allow process*)\n"
                  "(allow network*)\n"
                  "(allow sysctl-read)\n"
                  "(allow file-read*)\n"
                  "\n"
                  "; common special files and tty ioctls for interactive tools\n"
                  "(allow file-read-data\n"
                  "    (literal \"/dev/random\")\n"
                  "    (literal \"/dev/urandom\"))\n"
                  "(allow file-read-data file-write-data file-ioctl\n"
                  "    (literal \"/dev/null\")\n"
                  "    (literal \"/dev/tty\")\n"
                  "    (literal (param \"TTY\")))\n"
                  "\n"
                  "; the writable places are rooted under the launch directory plus any configured extras\n"
                  "(allow file-write*\n"
                  "    (subpath (param \"WORKDIR\"))\n"
                  "    (subpath (param \"HOME\"))\n"
                  "    (subpath (param \"TMPDIR\"))\n");
    for (size_t i = 0; i < extra_writable_dirs->len; ++i) {
        append_profile_subpath(&buf, extra_writable_dirs->items[i]);
    }
    strbuf_append(&buf, ")\n");
    return buf.data;
}

int main(int argc, char **argv) {
    if (cli_requests_help(argc, argv)) {
        print_help(stdout, base_name(argv[0]));
        return 0;
    }

    struct passwd *pw = getpwuid(getuid());
    const char *user_name = pw ? pw->pw_name : NULL;
    const char *host_home = pw ? pw->pw_dir : NULL;
    const char *pw_shell = (pw && pw->pw_shell && pw->pw_shell[0] != '\0') ? pw->pw_shell : NULL;
    const char *shell_path = pick_shell(getenv("SHELL"), pw_shell);
    const char *xdg_config_home = getenv("XDG_CONFIG_HOME");

    if (access("/usr/bin/sandbox-exec", X_OK) != 0) {
        dief("/usr/bin/sandbox-exec is unavailable on this macOS installation");
    }

    char cwd_buf[PATH_MAX];
    if (!getcwd(cwd_buf, sizeof(cwd_buf))) {
        die_errno("getcwd");
    }

    char workdir_buf[PATH_MAX];
    if (!realpath(cwd_buf, workdir_buf)) {
        die_errno("realpath(cwd)");
    }
    const char *workdir = workdir_buf;

    if (chdir(workdir) != 0) {
        die_errno("chdir");
    }

    struct strlist raw_extra_writable_dirs = {0};
    load_config_writable_dirs(&raw_extra_writable_dirs, host_home, xdg_config_home);

    int arg_index = 1;
    bool force_command = false;
    parse_cli_options(argc, argv, &raw_extra_writable_dirs, &arg_index, &force_command);

    struct strlist extra_writable_dirs = {0};
    resolve_writable_dirs(&extra_writable_dirs, &raw_extra_writable_dirs, host_home);

    refuse_redirected_regular_stdio(workdir, &extra_writable_dirs);

    char *sandbox_home = path_join(workdir, ".sbbash-home");
    char *sandbox_tmp = path_join(workdir, ".sbbash-tmp");
    bool home_ok = try_ensure_dir(sandbox_home, 0700);
    bool tmp_ok = try_ensure_dir(sandbox_tmp, 0700);

    if (!home_ok) {
        free(sandbox_home);
        sandbox_home = xstrdup(workdir);
    }
    if (!tmp_ok) {
        free(sandbox_tmp);
        sandbox_tmp = xstrdup(workdir);
    }

    char *histfile = NULL;
    const char *histname = history_file_name_for_shell(shell_path);
    if (strcmp(sandbox_home, workdir) == 0) {
        histfile = path_join(workdir, ".sbbash_history");
    } else {
        histfile = path_join(sandbox_home, histname);
    }

    sanitize_env(workdir, sandbox_home, sandbox_tmp, histfile, host_home, user_name, shell_path);
    close_extra_fds();

    const char *tty_path = ttyname(STDIN_FILENO);
    if (!tty_path || tty_path[0] == '\0') {
        tty_path = ttyname(STDOUT_FILENO);
    }
    if (!tty_path || tty_path[0] == '\0') {
        tty_path = ttyname(STDERR_FILENO);
    }
    if (!tty_path || tty_path[0] == '\0') {
        tty_path = "/dev/tty";
    }

    char *def_workdir = xmalloc(strlen("WORKDIR=") + strlen(workdir) + 1);
    char *def_home = xmalloc(strlen("HOME=") + strlen(sandbox_home) + 1);
    char *def_tmp = xmalloc(strlen("TMPDIR=") + strlen(sandbox_tmp) + 1);
    char *def_tty = xmalloc(strlen("TTY=") + strlen(tty_path) + 1);
    char *profile = build_profile_text(&extra_writable_dirs);
    sprintf(def_workdir, "WORKDIR=%s", workdir);
    sprintf(def_home, "HOME=%s", sandbox_home);
    sprintf(def_tmp, "TMPDIR=%s", sandbox_tmp);
    sprintf(def_tty, "TTY=%s", tty_path);

    if (force_command && arg_index == argc) {
        dief("`--` requires a command to run");
    }

    bool interactive_shell = (arg_index == argc);
    bool shell_arg_mode = (!interactive_shell && !force_command && argv[arg_index][0] == '-');
    int run_argc;
    if (interactive_shell) {
        run_argc = 2;
    } else if (shell_arg_mode) {
        run_argc = 1 + (argc - arg_index);
    } else {
        run_argc = argc - arg_index;
    }

    int total = 11 + run_argc + 1;
    char **child_argv = calloc((size_t)total, sizeof(char *));
    if (!child_argv) {
        dief("out of memory");
    }

    int i = 0;
    child_argv[i++] = "/usr/bin/sandbox-exec";
    child_argv[i++] = "-D";
    child_argv[i++] = def_workdir;
    child_argv[i++] = "-D";
    child_argv[i++] = def_home;
    child_argv[i++] = "-D";
    child_argv[i++] = def_tmp;
    child_argv[i++] = "-D";
    child_argv[i++] = def_tty;
    child_argv[i++] = "-p";
    child_argv[i++] = profile;

    if (interactive_shell) {
        child_argv[i++] = (char *)shell_path;
        child_argv[i++] = "-i";
    } else if (shell_arg_mode) {
        child_argv[i++] = (char *)shell_path;
        for (int j = arg_index; j < argc; ++j) {
            child_argv[i++] = argv[j];
        }
    } else {
        for (int j = arg_index; j < argc; ++j) {
            child_argv[i++] = argv[j];
        }
    }
    child_argv[i] = NULL;

    execv(child_argv[0], child_argv);
    die_errno("execv(/usr/bin/sandbox-exec)");
    return 111;
}
