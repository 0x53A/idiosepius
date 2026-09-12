"""Coordinate conventions shared by the kana data importers."""


def animcjk_screen_strokes(medians):
    # graphicsJaKana.txt uses font coordinates, unlike the SVG and our canvas.
    return [[[x, 900 - y] for x, y in stroke] for stroke in medians]


def self_test():
    # Independently read from the pinned svgsJaKana/12501.svg (フ).
    medians = [[[163, 594], [241, 582], [691, 682], [774, 648], [566, 332], [246, 53]]]
    assert animcjk_screen_strokes(medians) == [
        [[163, 306], [241, 318], [691, 218], [774, 252], [566, 568], [246, 847]]]
    assert medians[0][0] == [163, 594]
