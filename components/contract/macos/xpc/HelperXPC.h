#import <Foundation/Foundation.h>

NS_ASSUME_NONNULL_BEGIN

@protocol HelperXPCProtocol
- (void)getVersionWithReply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply NS_SWIFT_NAME(getVersion(reply:));
- (void)connect:(NSData *)request reply:(void (NS_SWIFT_SENDABLE ^)(NSData *_Nullable))reply;
- (void)submitOTPWithAttemptID:(NSString *)attemptID code:(NSString *)code reply:(void (NS_SWIFT_SENDABLE ^)(NSData *_Nullable))reply NS_SWIFT_NAME(submitOTP(attemptID:code:reply:));
- (void)disconnectWithAttemptID:(NSString *)attemptID reason:(NSString *)reason reply:(void (NS_SWIFT_SENDABLE ^)(NSData *_Nullable))reply NS_SWIFT_NAME(disconnect(attemptID:reason:reply:));
- (void)statusWithReply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply NS_SWIFT_NAME(status(reply:));
- (void)exitWhenIdleWithReply:(void (NS_SWIFT_SENDABLE ^)(BOOL))reply NS_SWIFT_NAME(exitWhenIdle(reply:));
- (void)bypassRoutesWithReply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply NS_SWIFT_NAME(bypassRoutes(reply:));
- (void)setBypassRoutes:(NSData *)request reply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply;
- (void)prepareUpdate:(NSData *)request reply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply;
- (void)installUpdate:(NSData *)request reply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply;
- (void)preparedUpdateWithReply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply NS_SWIFT_NAME(preparedUpdate(reply:));
- (void)discardPreparedUpdateWithReply:(void (NS_SWIFT_SENDABLE ^)(NSData *))reply NS_SWIFT_NAME(discardPreparedUpdate(reply:));
@end

@protocol HelperClientXPCProtocol
- (void)event:(NSData *)data;
- (void)output:(NSData *)data;
@end

NS_ASSUME_NONNULL_END
