#include <stdarg.h>
#include <stdio.h>
#include <stdlib.h>

typedef void (*fvm_oc_line_fn)(void *privdata, int level, const char *text);

static fvm_oc_line_fn line_fn;

void fvm_oc_set_line(fvm_oc_line_fn fn)
{
	line_fn = fn;
}

void fvm_oc_progress(void *privdata, int level, const char *fmt, ...)
{
	char text[1024];
	char *large;
	va_list args;
	va_list again;
	int length;

	va_start(args, fmt);
	va_copy(again, args);
	length = vsnprintf(text, sizeof(text), fmt, args);
	va_end(args);
	if (length < 0) {
		line_fn(privdata, level, fmt);
	} else if ((size_t)length < sizeof(text)) {
		line_fn(privdata, level, text);
	} else if ((large = malloc((size_t)length + 1)) != NULL) {
		vsnprintf(large, (size_t)length + 1, fmt, again);
		line_fn(privdata, level, large);
		free(large);
	} else {
		line_fn(privdata, level, text);
	}
	va_end(again);
}
