# Ironswift — العربية

## القائمة
menu-open-link-new-tab = فتح الرابط في لسان جديد
menu-copy-link = نسخ عنوان الرابط
menu-open-image-new-tab = فتح الصورة في لسان جديد
menu-copy-image = نسخ عنوان الصورة
menu-copy = نسخ
menu-back = رجوع
menu-forward = تقدّم
menu-reload = إعادة التحميل
menu-select-all = تحديد الكل
menu-find = البحث في الصفحة…
menu-save-pdf = حفظ بصيغة PDF…
menu-print = طباعة…
menu-view-source = عرض مصدر الصفحة
menu-inspect = فحص

## التنزيلات
dl-progress = { $pct }٪ · { $got } من { $total }
dl-so-far = { $got } حتى الآن
dl-done = اكتمل · { $size } · انقر للفتح
dl-failed = فشل: { $error }
dl-cancelled = أُلغي
dl-show-all = عرض كل التنزيلات
dl-open-folder = فتح مجلد التنزيلات

## تنبيهات
toast-zoom = التكبير { $pct }٪  ·  Ctrl+0 لإعادة الضبط
toast-printed = { $n ->
    [one] أُرسلت صفحة واحدة إلى الطابعة
    [two] أُرسلت صفحتان إلى الطابعة
    [few] أُرسلت { $n } صفحات إلى الطابعة
   *[other] أُرسلت { $n } صفحة إلى الطابعة
}
toast-pdf-saved = حُفظ { $file } ({ $n } ص) في التنزيلات
toast-pdf-no-printer = لم تستجب أي طابعة؛ حُفظ { $file } ({ $n } ص) في التنزيلات
toast-pdf-failed = تعذّر حفظ ملف PDF: { $error }

## النافذة والألسنة
window-loading = جارٍ تحميل { $target } — Ironswift
private-badge = خاص

## البحث في الصفحة
find-none = لا توجد نتائج
find-count = { $current } من { $total }
find-hint = Enter: التالي  ·  Shift+Enter: السابق  ·  Esc: إغلاق

## صفحات الخطأ
err-try-again = أعد المحاولة
err-settings = الإعدادات
err-details = التفاصيل
err-dns-title = تعذّر العثور على هذا الموقع
err-dns-detail = لم يُعثر على عنوان لـ { $host }.
err-dns-tip1 = تحقّق من كتابة العنوان.
err-dns-tip2 = تحقّق من اتصالك بالإنترنت.
err-dns-tip3 = ربما لم يعد الموقع موجودًا.
err-timeout-title = استغرق الموقع وقتًا طويلًا للرد
err-timeout-detail = لم يستجب { $host } في الوقت المحدد.
err-timeout-tip1 = أعد المحاولة بعد قليل؛ ربما يكون الموقع مشغولًا.
err-net-tip = تحقّق من اتصالك بالإنترنت.
err-cert-title = هذا الاتصال غير آمن
err-cert-detail = أرسل { $host } شهادة أمان لا يمكن لـ Ironswift الوثوق بها، لذلك لم تُحمَّل الصفحة. ربما يحاول أحدهم قراءة ما ترسله أو تغييره.
err-cert-tip1 = إن كان هذا موقعك أو موقع شركتك، فربما انتهت صلاحية شهادته أو لم تُثبَّت.
err-cert-tip2 = لا تُدخل كلمات مرور أو بيانات دفع في هذا الموقع.
err-refused-title = رفض الموقع الاتصال
err-refused-detail = لا يقبل { $host } الاتصالات.
err-refused-tip1 = ربما يكون الموقع معطّلًا أو لا يعمل على هذا المنفذ.
err-refused-tip2 = أعد المحاولة لاحقًا.
err-proxy-title = تعذّر الاتصال عبر الوكيل
err-proxy-detail = لم يسمح خادم الوكيل بمرور الطلب.
err-proxy-tip = تحقّق من الوكيل في الإعدادات (الشبكة).
err-type-title = لا يستطيع Ironswift عرض هذا الملف
err-type-detail = لا يمكن عرض هذا النوع من الملفات في لسان.
err-other-title = تعذّر تحميل هذه الصفحة
err-other-detail = حدث خطأ أثناء تحميل { $host }.
err-other-tip = أعد المحاولة باستخدام ↻.

## الإعدادات
set-language = اللغة
set-language-auto = مثل سطح المكتب
set-language-hint = لقوائم Ironswift ورسائله، واللغة التي تُبلَّغ بها المواقع. يسري بالكامل عند تشغيل Ironswift في المرة القادمة.
set-high-contrast = تباين عالٍ
set-high-contrast-hint = يعرض الصفحات والمتصفح بالأسود والأبيض والأصفر دون صور خلفية، لتسهيل القراءة.

## Screen readers (window, tabs, toolbar) and the settings page
a11y-tabs = علامات التبويب
a11y-toolbar = التنقّل
a11y-address = شريط العنوان والبحث
tab-untitled = علامة تبويب جديدة
button-back = رجوع
button-forward = تقدّم
button-reload = إعادة التحميل
settings-title = الإعدادات
settings-sub = تُحفظ على هذا الحاسوب فقط.
settings-saved = حُفظت الإعدادات.
settings-cache-cleared = مُسحت الملفات المخزّنة مؤقتًا (تحرّر { $mb } ميغابايت).
settings-startup = البدء والبحث
settings-home = الصفحة الرئيسية
settings-search-engine = محرك البحث
settings-restore-tabs = إعادة فتح علامات التبويب من المرة السابقة
settings-bookmarks-bar = إظهار شريط العلامات المرجعية
settings-memory = الذاكرة
settings-sleep = إسبات علامات التبويب في الخلفية بعد (دقائق، 0 = أبدًا)
settings-sleep-hint = تحرّر علامة التبويب المُسبتة صفحتها وتُعاد تحميلها عند العودة إليها.
settings-gpu = الرسم باستخدام بطاقة الرسوميات
settings-gpu-hint = تمرير أكثر سلاسة وعمل أقل للمعالج؛ يستخدم برنامج تشغيل الرسوميات نحو 40–70 ميغابايت إضافية من الذاكرة. يسري عند تشغيل Ironswift في المرة القادمة.
settings-site-processes = عزل المواقع: كل موقع في عملية خاصة به
settings-site-processes-hint = تعمل صفحات كل موقع في عملية منفصلة ومقيّدة لا يمكنها الوصول إلى ملفاتك أو إلى ملفات تعريف الارتباط لمواقع أخرى أو إلى الشبكة مباشرة. تستهلك ذاكرة أكثر. البحث في الصفحة وتحديد النص وبعض القوائم لا تعمل بعد في هذه الصفحات. يسري عند تشغيل Ironswift في المرة القادمة.
settings-privacy = الخصوصية
settings-tpc = مشاركة ملفات تعريف الارتباط مع المواقع المضمّنة
settings-tpc-hint = معطّل افتراضيًا: الموقع المضمّن في مواقع أخرى (شبكة إعلانات أو تتبّع، مشغّل فيديو) يحصل على مخزن منفصل لملفات تعريف الارتباط في كل موقع، فلا يستطيع تتبّعك.
settings-strip = إزالة وسوم التتبّع من الروابط
settings-strip-hint = utm_source وfbclid وgclid وما شابهها.
settings-gpc = إرسال إشارة Global Privacy Control
settings-gpc-hint = تطلب من المواقع ألا تبيع بياناتك أو تشاركها.
settings-clear-exit = حذف ملفات تعريف الارتباط وبيانات المواقع عند إغلاق Ironswift
settings-clear-exit-hint = يسجّل خروجك من المواقع في كل مرة؛ يُحتفظ بالسجل والعلامات المرجعية.
settings-cookies-allow = السماح دائمًا بملفات تعريف الارتباط من هذه المواقع
settings-cookies-allow-hint = حتى عندما تكون مضمّنة في مواقع أخرى (لمربعات تسجيل الدخول أو التعليقات التي تحتاجها).
settings-cookies-block = عدم السماح أبدًا بملفات تعريف الارتباط من هذه المواقع
settings-cookies-session = حذف ملفات تعريف الارتباط لهذه المواقع عند إغلاق Ironswift
settings-partition-hint = تحصل المواقع الأخرى المضمّنة في صفحة على مخزنها المنفصل لتلك الصفحة، فلا تستطيع تتبّعك من موقع إلى آخر.
settings-cookie-sites = مواقع لديها ملفات تعريف ارتباط ({ $count })
settings-clear-data = مسح بيانات التصفح الآن
settings-clear-data-hint = السجل وملفات تعريف الارتباط وبيانات المواقع والملفات المخزّنة مؤقتًا. تبقى العلامات المرجعية والإعدادات.
settings-private-hint = للتصفح دون الاحتفاظ بأي شيء، افتح نافذة خاصة بالضغط على Ctrl+Shift+N.
settings-blocking = الحظر والأمان
settings-block-ads = حظر الإعلانات
settings-block-ads-hint = EasyList: تُوقَف طلبات الإعلانات وتُخفى مساحاتها الفارغة. تُحمَّل الصفحات أسرع.
settings-block-trackers = حظر أدوات التتبّع
settings-block-trackers-hint = EasyPrivacy: أدوات التحليل وبكسلات التتبّع وبرامج بصمة الجهاز.
settings-no-blocking = إيقاف الحظر في هذه المواقع
settings-safe = تحذيري قبل المواقع الخطرة
settings-safe-hint = المواقع المدرجة في قائمة URLhaus للبرمجيات الخبيثة وقائمة Phishing Army. يجري الفحص على هذا الحاسوب؛ لا يُرسَل شيء.
settings-rfp = مقاومة بصمة الجهاز
settings-rfp-hint = ترى المواقع حجم شاشة شائعًا والخطوط القياسية وقراءات canvas متفاوتة قليلًا، فلا تستطيع التعرّف على حاسوبك من تفاصيله.
settings-update-lists = تحديث القوائم الآن
settings-permissions-hint = الكاميرا والميكروفون والموقع والإشعارات: تسأل المواقع أولًا.
settings-updates = التحديثات
settings-check-updates = البحث عن إصدارات جديدة مرة يوميًا
settings-check-now = البحث الآن
settings-appearance = المظهر
settings-min-font = أصغر حجم للخط (بكسل، 0 = بلا حد)
settings-min-font-hint = يكبّر الخط الصغير في الصفحات. Ctrl + وCtrl − يكبّران موقعًا كاملًا.
settings-reduce-motion = تقليل الحركة
settings-reduce-motion-hint = يطلب من المواقع تقليل الرسوم المتحركة إلى الحد الأدنى.
settings-default-browser = المتصفح الافتراضي
settings-make-default = جعل Ironswift متصفحي الافتراضي
settings-network = الشبكة
settings-proxy = الوكيل
settings-proxy-system = إعدادات النظام
settings-proxy-none = بلا وكيل
settings-proxy-manual = استخدام هذا الوكيل
settings-proxy-address = عنوان الوكيل (لخيار «استخدام هذا الوكيل»)
settings-proxy-hint = «إعدادات النظام» تتبع وكيل سطح المكتب والمتغيرات http_proxy / https_proxy / no_proxy. تُفحص المواقع الآمنة بمخزن شهادات الحاسوب.
settings-clear-cache = مسح الملفات المخزّنة مؤقتًا
settings-cache-size = (‏{ $mb } ميغابايت محفوظة لتحميل الصفحات أسرع)
settings-save = حفظ الإعدادات
settings-history = السجل
settings-bookmarks = العلامات المرجعية
settings-cookies-deleted = حُذفت ملفات تعريف الارتباط من { $site }.
settings-cookies-allowed = يمكن لـ { $site } استخدام ملفات تعريف الارتباط دائمًا.
settings-cookies-blocked = لم يعد بإمكان { $site } استخدام ملفات تعريف الارتباط (حُذفت ملفاته).
settings-cookies-session-set = ستُحذف ملفات تعريف الارتباط لـ { $site } عند إغلاق Ironswift.
settings-cookies-default = يتبع { $site } القواعد العادية لملفات تعريف الارتباط.
settings-permissions-reset = أُعيد ضبط أذونات { $site }.

## Find bar label
find-label = بحث
