import QtQuick 2.0
import Sailfish.Silica 1.0

// One row of a room list, shared by the chat list, the space overview and a
// single space's rooms. The width flows one way only — from the list, down
// through this delegate to its children — because mixing the two directions in
// one subtree is a binding loop that Qt breaks by zeroing a height, leaving a
// list that looks empty while the model is full. Keep it that way.
ListItem {
    id: roomItem

    // Optional text shown after the name, e.g. a space's count badge. Empty for
    // ordinary rooms.
    property string trailingText: ""

    contentHeight: Theme.itemSizeMedium

    Avatar {
        id: roomAvatar

        anchors {
            left: parent.left
            leftMargin: Theme.horizontalPageMargin
            verticalCenter: parent.verticalCenter
        }
        size: Theme.iconSizeMedium
        source: model.avatar || ""
        name: model.name
    }

    Column {
        anchors {
            left: roomAvatar.right
            leftMargin: Theme.paddingMedium
            right: unreadBadge.left
            rightMargin: Theme.paddingMedium
            verticalCenter: parent.verticalCenter
        }
        spacing: Theme.paddingSmall

        Row {
            width: parent.width
            spacing: Theme.paddingSmall

            Label {
                // Status icons follow the name in a fixed order — encrypted,
                // favourite, muted, low priority — so the reader always finds
                // each one in the same place instead of it moving as others
                // appear.
                width: Math.min(implicitWidth,
                                parent.width
                                - (lock.visible ? lock.width + Theme.paddingSmall : 0)
                                - (favouriteIcon.visible ? favouriteIcon.width + Theme.paddingSmall : 0)
                                - (mutedIcon.visible ? mutedIcon.width + Theme.paddingSmall : 0)
                                - (lowPriorityIcon.visible ? lowPriorityIcon.width + Theme.paddingSmall : 0)
                                - (badge.visible ? badge.width + Theme.paddingSmall : 0))
                truncationMode: TruncationMode.Fade
                color: roomItem.highlighted ? Theme.highlightColor : Theme.primaryColor
                textFormat: Text.PlainText
                text: model.name
            }

            Image {
                id: lock

                anchors.verticalCenter: parent.verticalCenter
                visible: model.encrypted
                source: "image://theme/icon-s-secure?" + (roomItem.highlighted
                                                          ? Theme.highlightColor
                                                          : Theme.secondaryColor)
            }

            Image {
                id: favouriteIcon

                anchors.verticalCenter: parent.verticalCenter
                visible: model.favourite === true
                source: "image://theme/icon-s-favorite?" + (roomItem.highlighted
                                                            ? Theme.highlightColor
                                                            : Theme.secondaryColor)
            }

            Image {
                id: mutedIcon

                // Muted and low priority are two different things — one silences
                // the room, the other sorts it to the bottom — and a room can be
                // either, or both, so each gets its own marker. The theme has no
                // icon-s- variant of the silent glyph, so the medium one is drawn
                // at the size the other markers in this row have.
                anchors.verticalCenter: parent.verticalCenter
                visible: model.muted === true
                width: Theme.iconSizeExtraSmall
                height: Theme.iconSizeExtraSmall
                sourceSize.width: Theme.iconSizeExtraSmall
                sourceSize.height: Theme.iconSizeExtraSmall
                source: "image://theme/icon-m-silent?" + (roomItem.highlighted
                                                          ? Theme.highlightColor
                                                          : Theme.secondaryColor)
            }

            Image {
                id: lowPriorityIcon

                anchors.verticalCenter: parent.verticalCenter
                visible: model.lowPriority === true
                source: "image://theme/icon-s-low-importance?" + (roomItem.highlighted
                                                                  ? Theme.highlightColor
                                                                  : Theme.secondaryColor)
            }

            Label {
                id: badge

                anchors.verticalCenter: parent.verticalCenter
                visible: text.length > 0
                text: roomItem.trailingText
                textFormat: Text.PlainText
                font.pixelSize: Theme.fontSizeExtraSmall
                color: roomItem.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            }
        }

        Label {
            width: parent.width
            font.pixelSize: Theme.fontSizeExtraSmall
            color: roomItem.highlighted ? Theme.secondaryHighlightColor : Theme.secondaryColor
            truncationMode: TruncationMode.Fade
            text: model.membership === "invited"
                  ? qsTr("Invitation")
                  : (model.space
                     ? qsTr("Space")
                     : (model.timestamp > 0
                        ? Format.formatDate(new Date(model.timestamp), Formatter.TimepointRelative)
                        : ""))
        }
    }

    // Unread indicator: mentions are what actually needs attention, so they get
    // the accent colour and the plain count stays quiet.
    Rectangle {
        id: unreadBadge

        anchors {
            right: parent.right
            rightMargin: Theme.horizontalPageMargin
            verticalCenter: parent.verticalCenter
        }
        visible: model.unread > 0 || model.mentions > 0
        width: Math.max(Theme.itemSizeExtraSmall / 2, badgeLabel.implicitWidth + Theme.paddingMedium)
        height: Theme.itemSizeExtraSmall / 2
        radius: height / 2
        color: model.mentions > 0 ? Theme.highlightColor : Theme.secondaryHighlightColor
        opacity: model.mentions > 0 ? 1.0 : 0.4

        Label {
            id: badgeLabel

            anchors.centerIn: parent
            font.pixelSize: Theme.fontSizeTiny
            color: Theme.highlightDimmerColor
            text: model.mentions > 0 ? model.mentions : model.unread
        }
    }
}
