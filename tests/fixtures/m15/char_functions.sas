/* M15.1 : Character functions — FIND, FINDC, COUNT, COUNTC, VERIFY, */
/* TRANSLATE, REVERSE, REPEAT, PROPCASE, COMPBL, SUBSTRN, CHAR, RANK, */
/* BYTE, WHICHC, CATQ.                                                  */

data work.test;
  /* FIND/FINDC — find substring or character */
  s = 'Hello World';
  find_o = find(s, 'o');         /* Should be 5 */
  find_wo = find(s, 'Wo', 6);    /* Should be 7 (search from pos 6) */
  find_not = find(s, 'xyz');     /* Should be 0 */
  findc_l = findc(s, 'lo');      /* Should be 3 (first l) */
  findc_x = findc(s, 'xyz');     /* Should be 0 */

  /* COUNT/COUNTC — count occurrences */
  s2 = 'abracadabra';
  count_a = count(s2, 'a');      /* Should be 5 */
  count_ab = count(s2, 'ab');    /* Should be 2 */
  count_x = count(s2, 'x');      /* Should be 0 */
  countc_vowel = countc(s2, 'aeiou');  /* Should be 5 (a,a,a,a,a) */

  /* VERIFY — find first char not in set */
  s3 = 'AAABBBCCC';
  verify_abc = verify(s3, 'ABC');    /* Should be 0 (all in set) */
  verify_ab = verify(s3, 'AB');      /* Should be 7 (first C) */
  s4 = 'xyz123';
  verify_letters = verify(s4, 'abcdefghijklmnopqrstuvwxyz');  /* 4 (first digit) */

  /* TRANSLATE — character translation */
  s5 = 'abc123';
  translate_result = translate(s5, 'ABC', 'abc');  /* Should be 'ABC123' */
  s6 = 'hello';
  translate_vouch = translate(s6, 'AEIOU', 'aeiou');  /* Should be 'hEllO' */

  /* REVERSE — reverse string */
  s7 = 'hello';
  reverse_s7 = reverse(s7);      /* Should be 'olleh' */
  s8 = 'A';
  reverse_s8 = reverse(s8);      /* Should be 'A' */

  /* REPEAT — repeat string */
  s9 = 'ab';
  repeat_3 = repeat(s9, 3);      /* Should be 'ababab' */
  repeat_0 = repeat(s9, 0);      /* Should be '' */
  repeat_neg = repeat(s9, -1);   /* Should be '' */

  /* PROPCASE — proper case */
  s10 = 'hello world';
  propcase_s10 = propcase(s10);  /* Should be 'Hello World' */
  s11 = 'hello-world-foo';
  propcase_dash = propcase(s11, '-');  /* Should be 'Hello-World-Foo' */

  /* COMPBL — compress blanks */
  s12 = '  hello   world  ';
  compbl_s12 = compbl(s12);      /* Should be 'hello world' (no leading/trailing) */

  /* SUBSTRN — substr without error on out-of-bounds */
  s13 = 'hello';
  substrn_ok = substrn(s13, 2, 3);   /* Should be 'ell' */
  substrn_oob = substrn(s13, 10, 2); /* Should be '' (no error) */

  /* CHAR/BYTE/RANK — ASCII/Unicode */
  char_65 = char(65);            /* Should be 'A' */
  char_32 = char(32);            /* Should be ' ' (space) */
  byte_97 = byte(97);            /* Should be 'a' */
  rank_a = rank('A');            /* Should be 65 */
  rank_hello = rank('hello');    /* Should be 104 (code of 'h') */

  /* WHICHC — find matching item */
  item = 'foo';
  which_found = whichc(item, 'bar', 'foo', 'baz');  /* Should be 2 */
  which_notfound = whichc(item, 'bar', 'baz', 'qux');  /* Should be 0 */
  which_first = whichc('a', 'a', 'b', 'a');  /* Should be 1 */

  /* CATQ — concatenate with quoting */
  catq_result = catq('|', 'hello', 'world');  /* Should be 'hello|world' */
  catq_quote = catq('|', 'hel|lo', 'world');  /* Should be '"hel|lo"|world' */

  output;
run;

title 'M15.1 Character Functions';
proc print data=work.test;
run;
