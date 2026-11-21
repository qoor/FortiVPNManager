#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@protocol HelperXPCProtocol
- (void)getVersionWithReply:(void (^)(NSData *))reply NS_SWIFT_NAME(getVersion(reply:));
- (void)connect:(NSData *)request reply:(void (^)(NSData *_Nullable))reply;
- (void)submitOTPWithAttemptID:(NSString *)attemptID code:(NSString *)code reply:(void (^)(NSData *_Nullable))reply NS_SWIFT_NAME(submitOTP(attemptID:code:reply:));
- (void)disconnectWithAttemptID:(NSString *)attemptID reason:(NSString *)reason reply:(void (^)(NSData *_Nullable))reply NS_SWIFT_NAME(disconnect(attemptID:reason:reply:));
- (void)statusWithReply:(void (^)(NSData *))reply NS_SWIFT_NAME(status(reply:));
- (void)exitWhenIdleWithReply:(void (^)(BOOL))reply NS_SWIFT_NAME(exitWhenIdle(reply:));
- (void)bypassRoutesWithReply:(void (^)(NSData *))reply NS_SWIFT_NAME(bypassRoutes(reply:));
- (void)setBypassRoutes:(NSData *)request reply:(void (^)(NSData *))reply;
- (void)prepareUpdate:(NSData *)request reply:(void (^)(NSData *))reply;
- (void)installUpdate:(NSData *)request reply:(void (^)(NSData *))reply;
- (void)preparedUpdateWithReply:(void (^)(NSData *))reply NS_SWIFT_NAME(preparedUpdate(reply:));
- (void)discardPreparedUpdateWithReply:(void (^)(NSData *))reply NS_SWIFT_NAME(discardPreparedUpdate(reply:));
@end

@protocol HelperClientXPCProtocol
- (void)event:(NSData *)data;
- (void)output:(NSData *)data;
@end

NS_ASSUME_NONNULL_END
