/*
 * The C23 strto* of glibc 2.38, for the release build against glibc 2.35.
 *
 * onnxruntime as ort downloads it (libonnxruntime.a, linked into tobiid) was
 * compiled against glibc 2.38 or newer, whose headers send strtol() and its
 * kin to __isoc23_strtol() and so on. glibc 2.35, which the packages are
 * built against so that they run on older systems, has only the classic
 * functions. The two differ only in the "0b" prefix C23 reads in bases 0 and
 * 2: for any other text they return the same.
 *
 * packaging/build.sh links this into every binary of the release build.
 * Hidden: no binary exports it, so libtobii.so in particular does not offer
 * these names to the processes that load it.
 */
#define _GNU_SOURCE
#include <locale.h>
#include <stdlib.h>

#define HIDDEN __attribute__((visibility("hidden")))

HIDDEN long __isoc23_strtol(const char *s, char **end, int base)
{
	return strtol(s, end, base);
}

HIDDEN long long __isoc23_strtoll(const char *s, char **end, int base)
{
	return strtoll(s, end, base);
}

HIDDEN unsigned long __isoc23_strtoul(const char *s, char **end, int base)
{
	return strtoul(s, end, base);
}

HIDDEN unsigned long long __isoc23_strtoull(const char *s, char **end, int base)
{
	return strtoull(s, end, base);
}

HIDDEN long long __isoc23_strtoll_l(const char *s, char **end, int base, locale_t loc)
{
	return strtoll_l(s, end, base, loc);
}

HIDDEN unsigned long long __isoc23_strtoull_l(const char *s, char **end, int base,
					      locale_t loc)
{
	return strtoull_l(s, end, base, loc);
}
