import UIKit

@_silgen_name("tish_ios_launch")
func tish_ios_launch()

@main
class AppDelegate: UIResponder, UIApplicationDelegate {
    func application(
        _ application: UIApplication,
        didFinishLaunchingWithOptions launchOptions: [UIApplication.LaunchOptionsKey: Any]? = nil
    ) -> Bool {
        tish_ios_launch()
        return true
    }
}
