#import <os/log.h>
#include <stdint.h>

os_log_t fvm_log_create(const char *subsystem, const char *category) {
    return os_log_create(subsystem, category);
}

void fvm_log_write(os_log_t log, uint8_t type, const char *message) {
    os_log_with_type(log, (os_log_type_t)type, "%{public}s", message);
}
