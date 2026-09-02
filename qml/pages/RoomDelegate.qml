import QtQuick 2.0
import Sailfish.Silica 1.0

// One row of a room list, shared by three views. The width flows one way only
// - from the list down - or Qt breaks the loop by zeroing a height.
ListItem {
    id: roomItem

    // Optional text shown after the name, e.g. a space's count badge. Empty for
    // ordinary rooms.
    property string trailingText: ""

    // Silica's relative timepoint gives no year, which reads as "this year" for a
    // conversation that stopped in 2025. Anything older is written out.
    readonly property string activityText: {
        if (!(model.timestamp > 0)) {
            return ""
        }
        var when = new Date(model.timestamp)
        return Format.formatDate(when,
                                 when.getFullYear() === new Date().getFullYear()
                                 ? Formatter.TimepointRelative
                                 : Formatter.DateMedium)
    }

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
                // Status icons follow the name in a fixed order - encrypted, favourite, muted,
                // low priority - so each is always found in the same place.
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

                // Muted and low priority are different things and a room can be both, so each
                // gets its own marker. The theme has no small silent glyph.
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
            // An upgraded room takes no new messages, so its last activity says the least
            // of anything that could stand here - and a dead room looks like a quiet one.
            text: model.membership === "invited"
                  ? qsTr("Invitation")
                  : (model.tombstoned === true
                     ? qsTr("Replaced by a new room")
                     : (model.space
                        ? qsTr("Space")
                        : roomItem.activityText))
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
        // Sized from the number so pill and digits grow together: the fixed-height
        // form kept the count near the smallest font and read as a speck.
        width: Math.max(height, badgeLabel.implicitWidth + Theme.paddingMedium)
        height: badgeLabel.implicitHeight + Theme.paddingSmall
        radius: height / 2
        // The one highlight fill the ambience guarantees under primary text, and only
        // at the pressed opacity - full strength swallowed the number on a light one.
        color: Theme.rgba(Theme.highlightBackgroundColor, Theme.highlightBackgroundOpacity)
        border.color: Theme.highlightColor
        border.width: model.mentions > 0 ? Math.max(2, Theme.paddingSmall / 3) : 0

        Label {
            id: badgeLabel

            anchors.centerIn: parent
            font.pixelSize: Theme.fontSizeSmall
            font.bold: true
            color: Theme.primaryColor
            // "20+" where the count reached the edge of what a sync carries: past that the
            // core knows only "at least". The ceiling lives in `LIST_TIMELINE_LIMIT`.
            text: {
                var count = model.mentions > 0 ? model.mentions : model.unread
                return model.unreadCapped ? count + "+" : count
            }
        }
    }
}
