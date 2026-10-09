Velox is a free desktop download manager: multi-connection downloads with pause, resume and crash recovery, browser integration, video downloads (HLS, DASH and many sites through yt-dlp), queues and a scheduler, FTP and SFTP, in English and Arabic.

## Download

| System | File |
|---|---|
| Windows 10 / 11 (64-bit) | `Velox-Download-Manager_{{version}}_x64-setup.exe` |
| Linux (x86-64) | `Velox-Download-Manager_{{version}}_amd64.AppImage` |

The Windows installer contains everything Velox needs (FFmpeg, yt-dlp, the browser extensions and the WebView2 runtime); nothing else has to be installed and no internet connection is needed to install it. On Linux, make the AppImage executable and run it.

SHA-256 checksums are in the `SHA256SUMS-*.txt` files (Windows: `Get-FileHash .\Velox-Download-Manager_{{version}}_x64-setup.exe`).

## Before you install

- **Not code-signed yet.** Windows SmartScreen shows "Windows protected your PC": choose **More info → Run anyway**.
- **Early release.** This build passed the automated tests on Windows and Linux, but it has not yet been checked on a clean Windows installation. Please report problems in [Issues](https://github.com/iraqi-man1/idm/issues).
- **No automatic updates** in this build: new versions are published here.

## Browser extension

The extension is not in the browser stores yet; it is installed with Velox.
In Velox open **Settings → Browser integration → Install browser extension** and follow the steps: in Chrome or Edge open `chrome://extensions` (or `edge://extensions`), turn on **Developer mode**, click **Load unpacked** and choose the folder Velox shows. Velox then shows **Connected**. Firefox can load it as a temporary add-on until a signed version is published.

## More

- [What is implemented and how it is tested](https://github.com/iraqi-man1/idm/blob/main/FEATURES.md)
- [Known issues](https://github.com/iraqi-man1/idm/blob/main/KNOWN_ISSUES.md)
- [Third-party software and licenses](https://github.com/iraqi-man1/idm/blob/main/THIRD_PARTY_NOTICES.md)

Velox is an independent project and is not affiliated with Internet Download Manager.

---

## بالعربية

Velox برنامج مجاني لإدارة التنزيلات: تنزيل بعدة اتصالات مع الإيقاف والاستئناف والاسترجاع بعد الأعطال، وتكامل مع المتصفح، وتنزيل الفيديو، وقوائم انتظار وجدولة، وFTP وSFTP، بواجهة عربية وإنجليزية.

- **ويندوز 10 / 11:** نزّل `Velox-Download-Manager_{{version}}_x64-setup.exe` وشغّله. المثبّت يحتوي على كل ما يحتاجه البرنامج ولا يحتاج إنترنت أثناء التثبيت.
- **غير موقّع رقمياً بعد:** سيظهر تحذير "Windows protected your PC"، اختر **More info ← Run anyway**.
- **إصدار مبكر:** نجح في الاختبارات الآلية على ويندوز ولينكس، ولم يُختبر بعد على نسخة ويندوز نظيفة.
- **إضافة المتصفح:** غير موجودة في المتجر بعد. من داخل Velox افتح **الإعدادات ← التكامل مع المتصفح ← تثبيت إضافة المتصفح** واتبع الخطوات (وضع المطوّر ثم "تحميل الإضافة غير المضغوطة").
