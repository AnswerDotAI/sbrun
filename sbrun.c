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

#ifndef SBBASH_DEFAULT_XDG_CONFIG_DIRS
#define SBBASH_DEFAULT_XDG_CONFIG_DIRS "/opt/homebrew/etc/xdg:/usr/local/etc/xdg:/etc/xdg"
#endif

#ifndef SBRUN_VERSION
#define SBRUN_VERSION "0.0.3"
#endif

static const char *DEFAULT_USER_CONFIG =
    "# Default writable allow-list for sbrun.\n"
    "#\n"
    "# These entries are intentionally narrower than \"your whole home directory\":\n"
    "# they try to cover common state, cache, and notebook/tool locations while\n"
    "# still avoiding broad write access to arbitrary files under $HOME.\n"
    "#\n"
    "# Optional paths that do not resolve to directories are ignored.\n"
    "# Add project- or tool-specific paths here or via -w/--writable.\n"
    "#\n"
    "# User config:\n"
    "#   $XDG_CONFIG_HOME/sbrun/config\n"
    "#   ~/.config/sbrun/config\n"
    "\n"
    "optional_writable_dir=/tmp\n"
    "optional_writable_dir=~/.cache\n"
    "optional_writable_dir=~/.config\n"
    "optional_writable_dir=~/.local/share\n"
    "optional_writable_dir=~/.local/state\n"
    "optional_writable_dir=~/.ipython\n"
    "optional_writable_dir=~/.jupyter\n"
    "optional_writable_dir=~/.matplotlib\n"
    "optional_writable_dir=~/.npm\n"
    "optional_writable_dir=~/.pnpm-store\n"
    "optional_writable_dir=~/.cargo\n"
    "optional_writable_dir=~/.rustup\n"
    "optional_writable_dir=~/.conda\n"
    "optional_writable_dir=~/Library/Caches\n"
    "optional_writable_dir=~/Library/Logs\n"
    "optional_writable_dir=~/Library/Jupyter\n"
    "optional_writable_dir=~/Library/Python\n"
    "optional_writable_dir=~/Library/Application Support/Jupyter\n";

typedef void *sandbox_params_t;
typedef void *sandbox_profile_t;

extern sandbox_params_t sandbox_create_params(void);
extern int sandbox_set_param(sandbox_params_t params, const char *key, const char *value);
extern void sandbox_free_params(sandbox_params_t params);
extern sandbox_profile_t sandbox_compile_string(const char *profile, sandbox_params_t params, char **errorbuf);
extern void sandbox_free_profile(sandbox_profile_t profile);
extern int sandbox_apply(sandbox_profile_t profile);

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
    fprintf(stderr, "sbrun: ");
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
            "Run commands under the macOS sandbox with writes confined to the\n"
            "current directory tree plus any configured extra writable paths.\n"
            "\n"
            "Options:\n"
            "  -h, --help           Show this help and exit\n"
            "  --version            Show version and exit\n"
            "  -w, --writable PATH  Allow writes to PATH; may be repeated\n"
            "  -e, --envdir VAR     Set VAR to .sbrun/VAR; may be repeated\n"
            "  -v, --unsetenv VAR   Remove VAR from the child environment; may be repeated\n"
            "  --                   Stop parsing %s options and force command mode\n"
            "\n"
            "Behavior:\n"
            "  With no command, start $SHELL as an interactive login shell.\n"
            "  If the first non-option argument starts with '-', pass arguments to $SHELL.\n"
            "  Otherwise run the given command directly.\n"
            "\n"
            "Config:\n"
            "  Global config: $XDG_CONFIG_DIRS/.../sbrun/config\n"
            "  User config:   $XDG_CONFIG_HOME/sbrun/config or ~/.config/sbrun/config\n"
            "  Format: writable_path=/path or optional_writable_path=/path\n"
            "          writable_dir=/path and optional_writable_dir=/path are also accepted\n"
            "  Envdir:   -e/--envdir VAR is CLI-only; VAR must be [A-Za-z_][A-Za-z0-9_]*\n"
            "  Unsetenv: -v/--unsetenv VAR is CLI-only; some names are reserved\n",
            prog,
            prog);
}

static void print_version(FILE *out, const char *prog) {
    fprintf(out, "%s %s\n", prog, SBRUN_VERSION);
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
        if (strcmp(argv[i], "-e") == 0 || strcmp(argv[i], "--envdir") == 0) {
            if (i + 1 < argc) {
                ++i;
            }
            continue;
        }
        if (strcmp(argv[i], "-v") == 0 || strcmp(argv[i], "--unsetenv") == 0) {
            if (i + 1 < argc) {
                ++i;
            }
            continue;
        }
        if (strncmp(argv[i], "--writable=", 11) == 0) {
            continue;
        }
        if (strncmp(argv[i], "--envdir=", 9) == 0) {
            continue;
        }
        if (strncmp(argv[i], "--unsetenv=", 11) == 0) {
            continue;
        }
        break;
    }
    return false;
}

static bool cli_requests_version(int argc, char **argv) {
    for (int i = 1; i < argc; ++i) {
        if (strcmp(argv[i], "--version") == 0) {
            return true;
        }
        if (strcmp(argv[i], "--") == 0) {
            return false;
        }
        if (strcmp(argv[i], "-w") == 0 || strcmp(argv[i], "--writable") == 0 ||
            strcmp(argv[i], "-e") == 0 || strcmp(argv[i], "--envdir") == 0 ||
            strcmp(argv[i], "-v") == 0 || strcmp(argv[i], "--unsetenv") == 0) {
            if (i + 1 < argc) {
                ++i;
            }
            continue;
        }
        if (strncmp(argv[i], "--writable=", 11) == 0 || strncmp(argv[i], "--envdir=", 9) == 0 ||
            strncmp(argv[i], "--unsetenv=", 11) == 0) {
            continue;
        }
        break;
    }
    return false;
}

static bool shell_is_bash(const char *shell_path) {
    return strcmp(base_name(shell_path), "bash") == 0;
}

static char *login_shell_argv0(const char *shell_path) {
    const char *name = base_name(shell_path);
    size_t n = strlen(name) + 2;
    char *out = xmalloc(n);
    out[0] = '-';
    memcpy(out + 1, name, n - 1);
    return out;
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

    char *config_dir = path_join(config_root, "sbrun");
    free(config_root);
    char *path = path_join(config_dir, "config");
    free(config_dir);
    return path;
}

static void ensure_directory_path(const char *path) {
    if (!path || path[0] == '\0') {
        return;
    }

    char *copy = xstrdup(path);
    for (char *p = copy + 1; *p; ++p) {
        if (*p != '/') {
            continue;
        }
        *p = '\0';
        if (mkdir(copy, 0700) != 0 && errno != EEXIST) {
            die_errno(copy);
        }
        *p = '/';
    }
    if (mkdir(copy, 0700) != 0 && errno != EEXIST) {
        die_errno(copy);
    }
    free(copy);
}

static void ensure_user_config_exists(const char *host_home, const char *xdg_config_home) {
    char *path = config_path(host_home, xdg_config_home);
    if (!path) {
        return;
    }
    if (access(path, F_OK) == 0) {
        free(path);
        return;
    }
    if (errno != ENOENT) {
        die_errno(path);
    }

    char *dir = xstrdup(path);
    char *slash = strrchr(dir, '/');
    if (!slash) {
        free(dir);
        free(path);
        return;
    }
    *slash = '\0';
    ensure_directory_path(dir);
    free(dir);

    FILE *fp = fopen(path, "w");
    if (!fp) {
        free(path);
        return;
    }
    if (fputs(DEFAULT_USER_CONFIG, fp) == EOF || fclose(fp) != 0) {
        die_errno(path);
    }
    free(path);
}

static void load_config_writable_dirs_from_path(struct strlist *dirs, struct strlist *optional_dirs, const char *path) {
    FILE *fp = fopen(path, "r");
    if (!fp) {
        if (errno == ENOENT) {
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
        if (strcmp(key, "writable_dir") == 0 || strcmp(key, "writable_path") == 0) {
            if (value[0] == '\0') {
                dief("%s:%zu: %s requires a path", path, lineno, key);
            }
            strlist_append_dup(dirs, value);
            continue;
        }
        if (strcmp(key, "optional_writable_dir") == 0 || strcmp(key, "optional_writable_path") == 0) {
            if (value[0] == '\0') {
                dief("%s:%zu: %s requires a path", path, lineno, key);
            }
            strlist_append_dup(optional_dirs, value);
            continue;
        }
        {
            dief("%s:%zu: unknown key %s", path, lineno, key);
        }
    }
    if (ferror(fp)) {
        dief("%s: %s", path, strerror(errno));
    }

    free(line);
    fclose(fp);
}

static void load_system_config_writable_dirs(struct strlist *dirs,
                                             struct strlist *optional_dirs,
                                             const char *xdg_config_dirs) {
    const char *roots = (xdg_config_dirs && xdg_config_dirs[0] != '\0')
        ? xdg_config_dirs
        : SBBASH_DEFAULT_XDG_CONFIG_DIRS;

    const char *start = roots;
    while (*start) {
        const char *end = strchr(start, ':');
        size_t len = end ? (size_t)(end - start) : strlen(start);
        if (len > 0 && start[0] == '/') {
            char *root = xmalloc(len + 1);
            memcpy(root, start, len);
            root[len] = '\0';
            char *config_dir = path_join(root, "sbrun");
            char *path = path_join(config_dir, "config");
            load_config_writable_dirs_from_path(dirs, optional_dirs, path);
            free(path);
            free(config_dir);
            free(root);
        }
        if (!end) {
            break;
        }
        start = end + 1;
    }
}

static void load_user_config_writable_dirs(struct strlist *dirs,
                                           struct strlist *optional_dirs,
                                           const char *host_home,
                                           const char *xdg_config_home) {
    char *path = config_path(host_home, xdg_config_home);
    if (!path) {
        return;
    }
    load_config_writable_dirs_from_path(dirs, optional_dirs, path);
    free(path);
}

static bool valid_env_name(const char *name) {
    if (!name || name[0] == '\0') {
        return false;
    }
    if (!(isalpha((unsigned char)name[0]) || name[0] == '_')) {
        return false;
    }
    for (size_t i = 1; name[i] != '\0'; ++i) {
        if (!(isalnum((unsigned char)name[i]) || name[i] == '_')) {
            return false;
        }
    }
    return true;
}

static bool unsetenv_name_is_reserved(const char *name) {
    static const char *reserved[] = {
        "PATH",
        "PWD",
        "HOME",
        "TMPDIR",
        "HISTFILE",
        "SHELL",
        "SBRUN_ACTIVE",
        "USER",
        "LOGNAME",
        "TERM",
        "LANG",
        "LC_ALL",
        "LC_CTYPE",
        "BASH_SILENCE_DEPRECATION_WARNING",
    };

    if (strncmp(name, "SBBASH_", 7) == 0) {
        return true;
    }

    for (size_t i = 0; i < sizeof(reserved) / sizeof(reserved[0]); ++i) {
        if (strcmp(name, reserved[i]) == 0) {
            return true;
        }
    }
    return false;
}

static void parse_cli_options(int argc,
                              char **argv,
                              struct strlist *dirs,
                              struct strlist *envdir_vars,
                              struct strlist *unsetenv_vars,
                              int *arg_index,
                              bool *force_command) {
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
                dief("%s requires a path argument", argv[i]);
            }
            strlist_append_dup(dirs, argv[i + 1]);
            i += 2;
            continue;
        }
        if (strcmp(argv[i], "-e") == 0 || strcmp(argv[i], "--envdir") == 0) {
            if (i + 1 >= argc) {
                dief("%s requires an environment variable name", argv[i]);
            }
            if (!valid_env_name(argv[i + 1])) {
                dief("invalid envdir variable name %s", argv[i + 1]);
            }
            if (strlist_contains(unsetenv_vars, argv[i + 1])) {
                dief("cannot use --envdir and --unsetenv for the same variable %s", argv[i + 1]);
            }
            strlist_append_unique_owned(envdir_vars, xstrdup(argv[i + 1]));
            i += 2;
            continue;
        }
        if (strcmp(argv[i], "-v") == 0 || strcmp(argv[i], "--unsetenv") == 0) {
            if (i + 1 >= argc) {
                dief("%s requires an environment variable name", argv[i]);
            }
            if (!valid_env_name(argv[i + 1])) {
                dief("invalid unsetenv variable name %s", argv[i + 1]);
            }
            if (unsetenv_name_is_reserved(argv[i + 1])) {
                dief("cannot unset reserved environment variable %s", argv[i + 1]);
            }
            if (strlist_contains(envdir_vars, argv[i + 1])) {
                dief("cannot use --envdir and --unsetenv for the same variable %s", argv[i + 1]);
            }
            strlist_append_unique_owned(unsetenv_vars, xstrdup(argv[i + 1]));
            i += 2;
            continue;
        }
        if (strncmp(argv[i], "--writable=", 11) == 0) {
            const char *value = argv[i] + 11;
            if (value[0] == '\0') {
                dief("--writable requires a path argument");
            }
            strlist_append_dup(dirs, value);
            ++i;
            continue;
        }
        if (strncmp(argv[i], "--envdir=", 9) == 0) {
            const char *value = argv[i] + 9;
            if (value[0] == '\0') {
                dief("--envdir requires an environment variable name");
            }
            if (!valid_env_name(value)) {
                dief("invalid envdir variable name %s", value);
            }
            if (strlist_contains(unsetenv_vars, value)) {
                dief("cannot use --envdir and --unsetenv for the same variable %s", value);
            }
            strlist_append_unique_owned(envdir_vars, xstrdup(value));
            ++i;
            continue;
        }
        if (strncmp(argv[i], "--unsetenv=", 11) == 0) {
            const char *value = argv[i] + 11;
            if (value[0] == '\0') {
                dief("--unsetenv requires an environment variable name");
            }
            if (!valid_env_name(value)) {
                dief("invalid unsetenv variable name %s", value);
            }
            if (unsetenv_name_is_reserved(value)) {
                dief("cannot unset reserved environment variable %s", value);
            }
            if (strlist_contains(envdir_vars, value)) {
                dief("cannot use --envdir and --unsetenv for the same variable %s", value);
            }
            strlist_append_unique_owned(unsetenv_vars, xstrdup(value));
            ++i;
            continue;
        }
        break;
    }

    *arg_index = i;
}

static char *resolve_writable_path(const char *path, const char *host_home, bool optional, bool *is_dir) {
    char *expanded = expand_home_path(path, host_home);
    char resolved[PATH_MAX];
    if (!realpath(expanded, resolved)) {
        int err = errno;
        free(expanded);
        if (optional && (err == ENOENT || err == ENOTDIR)) {
            return NULL;
        }
        dief("additional writable path %s: %s", path, strerror(err));
    }
    free(expanded);

    struct stat st;
    if (stat(resolved, &st) != 0) {
        if (optional && errno == ENOENT) {
            return NULL;
        }
        dief("additional writable path %s: %s", path, strerror(errno));
    }
    if (S_ISDIR(st.st_mode)) {
        *is_dir = true;
        return xstrdup(resolved);
    }
    if (!S_ISREG(st.st_mode)) {
        if (optional) {
            return NULL;
        }
        dief("additional writable path %s resolves to %s, which is not a regular file or directory", path, resolved);
    }
    *is_dir = false;
    return xstrdup(resolved);
}

static void resolve_writable_paths(struct strlist *resolved_dirs,
                                   struct strlist *resolved_files,
                                   const struct strlist *raw_dirs,
                                   const char *host_home,
                                   bool optional) {
    for (size_t i = 0; i < raw_dirs->len; ++i) {
        bool is_dir = false;
        char *resolved = resolve_writable_path(raw_dirs->items[i], host_home, optional, &is_dir);
        if (!resolved) {
            continue;
        }
        if (is_dir) {
            strlist_append_unique_owned(resolved_dirs, resolved);
        } else {
            strlist_append_unique_owned(resolved_files, resolved);
        }
    }
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

static void ensure_real_directory(const char *path) {
    struct stat st;
    if (lstat(path, &st) == 0) {
        if (S_ISLNK(st.st_mode)) {
            dief("%s exists and is a symlink; refusing to use it", path);
        }
        if (!S_ISDIR(st.st_mode)) {
            dief("%s exists and is not a directory", path);
        }
        return;
    }
    if (errno != ENOENT) {
        die_errno(path);
    }
    if (mkdir(path, 0700) == 0) {
        return;
    }
    if (errno != EEXIST) {
        die_errno(path);
    }
    if (lstat(path, &st) != 0) {
        die_errno(path);
    }
    if (S_ISLNK(st.st_mode) || !S_ISDIR(st.st_mode)) {
        dief("%s exists and is not a real directory", path);
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

static bool path_is_allowed_write_target(const char *path,
                                         const char *workdir,
                                         const struct strlist *extra_writable_dirs,
                                         const struct strlist *extra_writable_files) {
    if (path_is_within(path, workdir)) {
        return true;
    }
    for (size_t i = 0; i < extra_writable_dirs->len; ++i) {
        if (path_is_within(path, extra_writable_dirs->items[i])) {
            return true;
        }
    }
    for (size_t i = 0; i < extra_writable_files->len; ++i) {
        if (strcmp(path, extra_writable_files->items[i]) == 0) {
            return true;
        }
    }
    return false;
}
#endif

static void refuse_redirected_regular_stdio(const char *workdir,
                                            const struct strlist *extra_writable_dirs,
                                            const struct strlist *extra_writable_files) {
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

        if (!path_is_allowed_write_target(final_path, workdir, extra_writable_dirs, extra_writable_files)) {
            dief("fd %d is redirected to %s outside allowed writable paths; refusing to start (set SBBASH_ALLOW_STDIO_REDIRECTS=1 to override)",
                 fd,
                 final_path);
        }
    }
#else
    (void)workdir;
    (void)extra_writable_dirs;
    (void)extra_writable_files;
#endif
}

static void sanitize_env(const char *workdir,
                         const char *tmpdir,
                         const char *histfile,
                         const char *host_home,
                         const char *user_name,
                         const char *shell_path,
                         const char *envdir_root,
                         const struct strlist *envdir_vars,
                         const struct strlist *unsetenv_vars) {
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
    if (host_home && host_home[0] != '\0') {
        setenv("HOME", host_home, 1);
    }
    setenv("TMPDIR", tmpdir, 1);
    if (histfile) {
        setenv("HISTFILE", histfile, 1);
    } else {
        unsetenv("HISTFILE");
    }
    setenv("SHELL", shell_path, 1);
    setenv("SBBASH_WORKDIR", workdir, 1);
    setenv("SBRUN_ACTIVE", "1", 1);
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

    if (envdir_root) {
        for (size_t i = 0; i < envdir_vars->len; ++i) {
            char *envdir = path_join(envdir_root, envdir_vars->items[i]);
            setenv(envdir_vars->items[i], envdir, 1);
            free(envdir);
        }
    }

    for (size_t i = 0; i < unsetenv_vars->len; ++i) {
        unsetenv(unsetenv_vars->items[i]);
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

static void append_profile_literal(struct strbuf *buf, const char *path) {
    char *escaped = escape_sandbox_string(path);
    strbuf_appendf(buf, "    (literal \"%s\")\n", escaped);
    free(escaped);
}

static char *build_profile_text(const struct strlist *extra_writable_dirs,
                                const struct strlist *extra_writable_files,
                                bool allow_histfile) {
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
                  "; the writable places are rooted under the launch directory and any configured extras\n"
                  "(allow file-write*\n"
                  "    (subpath (param \"WORKDIR\"))\n");
    for (size_t i = 0; i < extra_writable_dirs->len; ++i) {
        append_profile_subpath(&buf, extra_writable_dirs->items[i]);
    }
    strbuf_append(&buf, ")\n");
    if (allow_histfile || extra_writable_files->len > 0) {
        strbuf_append(&buf, "(allow file-write*\n");
        if (allow_histfile) {
            strbuf_append(&buf, "    (literal (param \"HISTFILE\"))\n");
        }
        for (size_t i = 0; i < extra_writable_files->len; ++i) {
            append_profile_literal(&buf, extra_writable_files->items[i]);
        }
        strbuf_append(&buf, ")\n");
    }
    return buf.data;
}

static void set_sandbox_param_or_die(sandbox_params_t params, const char *key, const char *value) {
    if (sandbox_set_param(params, key, value) != 0) {
        die_errno("sandbox_set_param");
    }
}

static sandbox_profile_t compile_sandbox_profile_or_die(const char *profile_text, sandbox_params_t params) {
    char *errorbuf = NULL;
    sandbox_profile_t profile = sandbox_compile_string(profile_text, params, &errorbuf);
    if (!profile) {
        if (errorbuf && errorbuf[0] != '\0') {
            dief("%s", errorbuf);
        }
        dief("sandbox profile compilation failed");
    }
    return profile;
}

int main(int argc, char **argv) {
    struct passwd *pw = getpwuid(getuid());
    const char *host_home = pw ? pw->pw_dir : NULL;
    const char *xdg_config_home = getenv("XDG_CONFIG_HOME");
    ensure_user_config_exists(host_home, xdg_config_home);

    if (cli_requests_help(argc, argv)) {
        print_help(stdout, base_name(argv[0]));
        return 0;
    }
    if (cli_requests_version(argc, argv)) {
        print_version(stdout, base_name(argv[0]));
        return 0;
    }

    const char *user_name = pw ? pw->pw_name : NULL;
    const char *pw_shell = (pw && pw->pw_shell && pw->pw_shell[0] != '\0') ? pw->pw_shell : NULL;
    const char *shell_path = pick_shell(getenv("SHELL"), pw_shell);
    const char *xdg_config_dirs = getenv("XDG_CONFIG_DIRS");

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
    struct strlist raw_optional_writable_dirs = {0};
    struct strlist envdir_vars = {0};
    struct strlist unsetenv_vars = {0};
    load_system_config_writable_dirs(&raw_extra_writable_dirs, &raw_optional_writable_dirs, xdg_config_dirs);
    ensure_user_config_exists(host_home, xdg_config_home);
    load_user_config_writable_dirs(&raw_extra_writable_dirs, &raw_optional_writable_dirs, host_home, xdg_config_home);

    int arg_index = 1;
    bool force_command = false;
    parse_cli_options(argc, argv, &raw_extra_writable_dirs, &envdir_vars, &unsetenv_vars, &arg_index, &force_command);

    struct strlist extra_writable_dirs = {0};
    struct strlist extra_writable_files = {0};
    resolve_writable_paths(&extra_writable_dirs, &extra_writable_files, &raw_extra_writable_dirs, host_home, false);
    resolve_writable_paths(&extra_writable_dirs, &extra_writable_files, &raw_optional_writable_dirs, host_home, true);

    refuse_redirected_regular_stdio(workdir, &extra_writable_dirs, &extra_writable_files);

    const char *tmpdir = "/tmp";
    char *histfile = NULL;
    if (host_home && host_home[0] != '\0') {
        histfile = path_join(host_home, history_file_name_for_shell(shell_path));
    }

    char *envdir_root = NULL;
    if (envdir_vars.len > 0) {
        envdir_root = path_join(workdir, ".sbrun");
        ensure_real_directory(envdir_root);
        for (size_t i = 0; i < envdir_vars.len; ++i) {
            char *envdir = path_join(envdir_root, envdir_vars.items[i]);
            ensure_real_directory(envdir);
            free(envdir);
        }
    }

    sanitize_env(workdir, tmpdir, histfile, host_home, user_name, shell_path, envdir_root, &envdir_vars, &unsetenv_vars);
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

    char *profile = build_profile_text(&extra_writable_dirs, &extra_writable_files, histfile != NULL);
    sandbox_params_t params = sandbox_create_params();
    if (!params) {
        die_errno("sandbox_create_params");
    }
    set_sandbox_param_or_die(params, "WORKDIR", workdir);
    set_sandbox_param_or_die(params, "TTY", tty_path);
    if (histfile) {
        set_sandbox_param_or_die(params, "HISTFILE", histfile);
    }
    sandbox_profile_t compiled_profile = compile_sandbox_profile_or_die(profile, params);

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

    char **run_argv = calloc((size_t)run_argc + 1, sizeof(char *));
    if (!run_argv) {
        dief("out of memory");
    }

    int i = 0;
    const char *exec_path = NULL;
    if (interactive_shell) {
        exec_path = shell_path;
        run_argv[i++] = login_shell_argv0(shell_path);
        run_argv[i++] = "-i";
    } else if (shell_arg_mode) {
        exec_path = shell_path;
        run_argv[i++] = (char *)shell_path;
        for (int j = arg_index; j < argc; ++j) {
            run_argv[i++] = argv[j];
        }
    } else {
        exec_path = argv[arg_index];
        for (int j = arg_index; j < argc; ++j) {
            run_argv[i++] = argv[j];
        }
    }
    run_argv[i] = NULL;

    if (sandbox_apply(compiled_profile) != 0) {
        die_errno("sandbox_apply");
    }

    sandbox_free_params(params);
    sandbox_free_profile(compiled_profile);

    execvp(exec_path, run_argv);
    dief("execvp(%s): %s", exec_path, strerror(errno));
    return 111;
}
