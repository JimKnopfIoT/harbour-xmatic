# xmatic — Sailfish OS qmake project.
#
# The Matrix protocol core is a Rust static library that is cross-built OUTSIDE
# this build, on the host, by scripts/build-core.sh. qmake only links it. Run
# that script once before the first build and again after touching core/.

TARGET = harbour-xmatic

CONFIG += sailfishapp sailfishapp_i18n c++11

QT += network dbus multimedia

# The media half of a call: Matrix carries only the signalling.
#
# pkg-config is queried directly rather than through link_pkgconfig: enabling
# that feature here resolves PKGCONFIG before sailfishapp.prf has added its own
# entry, which silently drops -lsailfishapp from the link line.
GST_MODULES = gstreamer-1.0 gstreamer-sdp-1.0 gstreamer-webrtc-1.0 gstreamer-video-1.0
QMAKE_CXXFLAGS += $$system(pkg-config --cflags $$GST_MODULES)
QMAKE_CXXFLAGS += $$system(pkg-config --cflags libpulse)

# Keep the build machine out of the binary: __FILE__ and Qt's assertions would
# otherwise embed the absolute source path of whoever compiled it.
QMAKE_CXXFLAGS += -ffile-prefix-map=$$PWD=/build

# Binary hardening. FORTIFY needs optimisation, which the release build has.
# stack-protector-strong guards return addresses; full RELRO plus BIND_NOW
# makes the GOT read-only after load; the binary is already position
# independent. These raise the cost of exploiting a decoder bug in a received
# image or video, which is the app's main untrusted-input surface.
QMAKE_CXXFLAGS += -D_FORTIFY_SOURCE=2 -fstack-protector-strong
QMAKE_LFLAGS += -Wl,-z,relro -Wl,-z,now
LIBS += $$system(pkg-config --libs $$GST_MODULES)
LIBS += $$system(pkg-config --libs libpulse)

SOURCES += \
    src/harbour-xmatic.cpp \
    src/difflistmodel.cpp \
    src/matrixbridge.cpp \
    src/roomlistmodel.cpp \
    src/roomsortmodel.cpp \
    src/directorymodel.cpp \
    src/membermodel.cpp \
    src/timelinemodel.cpp \
    src/callengine.cpp \
    src/camerasource.cpp \
    src/callaudiorouter.cpp \
    src/videostream.cpp \
    src/voicerecorder.cpp

HEADERS += \
    src/difflistmodel.h \
    src/matrixbridge.h \
    src/roomlistmodel.h \
    src/roomsortmodel.h \
    src/directorymodel.h \
    src/membermodel.h \
    src/timelinemodel.h \
    src/callengine.h \
    src/camerasource.h \
    src/callaudiorouter.h \
    src/videostream.h \
    src/voicerecorder.h

# cbindgen writes the C ABI header here (generated, not tracked).
INCLUDEPATH += src/generated

# --- Rust core --------------------------------------------------------------
# One Rust triple per RPM architecture; QT_ARCH names the target the qmake
# in the build chroot was built for.
equals(QT_ARCH, arm64): RUST_TARGET = aarch64-unknown-linux-gnu
else:equals(QT_ARCH, arm): RUST_TARGET = armv7-unknown-linux-gnueabihf
else:equals(QT_ARCH, i386): RUST_TARGET = i586-unknown-linux-gnu
else: error("Unsupported QT_ARCH: $$QT_ARCH")
CORE_LIB = $$PWD/core/target/$$RUST_TARGET/release/libxmatic_core.a

!exists($$CORE_LIB) {
    error("Rust core not built: $$CORE_LIB is missing. Run scripts/build-core.sh first.")
}

LIBS += $$CORE_LIB -lpthread -ldl -lm -lrt
PRE_TARGETDEPS += $$CORE_LIB

# Installed to /usr/share/icons/hicolor/<size>/apps/ (see sailfishapp.prf).
SAILFISHAPP_ICONS = 86x86 108x108 128x128 172x172

# Lets the share dialog start the app when it is not already running. The file
# name has to be the D-Bus name, which Sailjail fixes; see the file itself.
dbusservice.files = org.xmatic.xmatic.service
dbusservice.path = /usr/share/dbus-1/services
INSTALLS += dbusservice

# Bilingual: English source strings plus German. libsailfishapp loads the .qm
# matching the device locale, otherwise the source strings are used.
TRANSLATIONS += translations/harbour-xmatic-de.ts

DISTFILES += \
    LICENSE \
    README.md \
    harbour-xmatic.desktop \
    rpm/harbour-xmatic.spec \
    icons/xmatic-logo.svg \
    qml/harbour-xmatic.qml \
    qml/cover/CoverPage.qml \
    qml/pages/AccountPage.qml \
    qml/pages/LoginPage.qml \
    qml/pages/AboutPage.qml \
    qml/pages/RoomListPage.qml \
    qml/pages/RoomDelegate.qml \
    qml/pages/SpacesPage.qml \
    qml/pages/SpacePage.qml \
    qml/pages/CreateSpaceDialog.qml \
    qml/pages/AddToSpacePage.qml \
    qml/pages/MoveToSpacePage.qml \
    qml/pages/RoomPage.qml \
    qml/pages/VerificationPage.qml \
    qml/pages/EncryptionPage.qml \
    qml/pages/LogoutDialog.qml \
    qml/pages/ImageViewPage.qml \
    qml/pages/ForwardPage.qml \
    qml/pages/ShareToRoomPage.qml \
    qml/pages/MessageActionsPage.qml \
    qml/pages/Avatar.qml \
    qml/pages/VideoPage.qml \
    qml/pages/JoinRoomDialog.qml \
    qml/pages/NewChatDialog.qml \
    qml/pages/CreateRoomDialog.qml \
    qml/pages/InviteToRoomDialog.qml \
    qml/pages/DirectoryPage.qml \
    qml/pages/AddDirectoryServerDialog.qml \
    qml/pages/PinnedMessagesPage.qml \
    qml/pages/MemberListPage.qml \
    qml/pages/CallPage.qml

lupdate_only {
    SOURCES += qml/*.qml qml/cover/*.qml qml/pages/*.qml
}
