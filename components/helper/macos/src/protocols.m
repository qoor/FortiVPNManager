#import "HelperXPC.h"

Protocol *fvm_helper_protocols(void) {
    return @protocol(HelperXPCProtocol) ?: @protocol(HelperClientXPCProtocol);
}
