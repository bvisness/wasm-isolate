package parser

import "slices"

func _instr_block_1(s *Stream) []*OSource_Phrase[OInstr_] {
	block := _instr_block__2(s, nil)
	slices.Reverse(block)
	return block
}

func _instr_block__2(s *Stream, es []*OSource_Phrase[OInstr_]) []*OSource_Phrase[OInstr_] {
	b := _peek_1(s)
	if b == nil || *b == 0x05 || *b == 0x0b {
		return es
	}
	pos := _pos_1(s)
	e := _instr_1(s)
	return _instr_block__2(s, append([]*OSource_Phrase[OInstr_]{_operatorAtAt_2(e, _region_3(s, pos, pos))}, es...))
}
