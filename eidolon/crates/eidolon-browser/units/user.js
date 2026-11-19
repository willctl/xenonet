// What an unattended browser needs and nothing else. Nothing here changes
// how the browser looks to a site.
user_pref("browser.shell.checkDefaultBrowser", false);
user_pref("browser.aboutwelcome.enabled", false);
user_pref("browser.startup.homepage_override.mstone", "ignore");
user_pref("startup.homepage_welcome_url", "");
user_pref("startup.homepage_welcome_url.additional", "");
user_pref("browser.sessionstore.resume_from_crash", false);
user_pref("datareporting.policy.dataSubmissionEnabled", false);
user_pref("datareporting.healthreport.uploadEnabled", false);
user_pref("toolkit.telemetry.reportingpolicy.firstRun", false);
user_pref("app.update.auto", false);
user_pref("app.update.checkInstallTime", false);
user_pref("app.update.service.enabled", false);
user_pref("extensions.update.enabled", false);
// Downloads land in one folder of the cell. Uploads are read from
// /home/eidolon/files/uploads: a file input is given that path.
user_pref("browser.download.folderList", 2);
user_pref("browser.download.dir", "/home/eidolon/files/downloads");
user_pref("browser.download.useDownloadDir", true);
user_pref("browser.download.always_ask_before_handling_new_types", false);
