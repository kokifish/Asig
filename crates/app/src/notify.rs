//! macOS 系统通知(UserNotifications framework):状态转入 NeedsDeci/Error 等时弹通知,
//! 让用户全屏干活也能被叫回。启动请求授权(首次弹系统对话框);授权未给则 send 静默 no-op。

use block2::RcBlock;
use objc2::MainThreadMarker;
use objc2::runtime::Bool;
use objc2_foundation::{NSError, NSString};
use objc2_user_notifications::{
    UNAuthorizationOptions, UNMutableNotificationContent, UNNotificationRequest,
    UNUserNotificationCenter,
};

/// 请求通知授权(alert + sound)。首次调系统弹对话框;已授权/拒绝则 no-op。主线程。
pub fn request_authorization() {
    let _mtm = MainThreadMarker::new().expect("notify::request_authorization 须主线程");
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let options = UNAuthorizationOptions::Alert.union(UNAuthorizationOptions::Sound);
    let block: RcBlock<dyn Fn(Bool, *mut NSError)> = RcBlock::new(|_g: Bool, _e: *mut NSError| {});
    center.requestAuthorizationWithOptions_completionHandler(options, &block);
}

/// 发即时通知(title + body)。identifier 固定("asig-status"),新通知覆盖旧的(通知中心不堆积)。
/// 授权未给则系统不显示(静默 no-op)。主线程。
pub fn send(title: &str, body: &str) {
    let _mtm = MainThreadMarker::new().expect("notify::send 须主线程");
    let center = UNUserNotificationCenter::currentNotificationCenter();
    let content = UNMutableNotificationContent::new();
    content.setTitle(&NSString::from_str(title));
    content.setBody(&NSString::from_str(body));
    let request = UNNotificationRequest::requestWithIdentifier_content_trigger(
        &NSString::from_str("asig-status"),
        &content,
        None,
    );
    let block: RcBlock<dyn Fn(*mut NSError)> = RcBlock::new(|_e: *mut NSError| {});
    center.addNotificationRequest_withCompletionHandler(&request, Some(&block));
}
