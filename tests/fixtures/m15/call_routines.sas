/* M15.6: CALL routines — CALL MISSING, CALL EXECUTE, CALL SORTN/SORTC,    */
/* CALL SYMPUTX, CALL CATS/SCAN, CALL LABEL, CALL VNAME.                   */

data work.test;
  length name $32 lbl $40 result $20 word $10 c1 $1 c2 $1 c3 $1;

  /* CALL MISSING — set variables to missing */
  x = 42;
  y = 'hello';
  call missing(x, y);
  /* x and y should now be missing */

  /* CALL VNAME — get variable name by reference */
  var = 99;
  call vname(var, name);  /* name should be 'var' */

  /* CALL LABEL — get variable label (falls back to name when none) */
  age = 25;
  label age = 'Age in years';
  call label(age, lbl);   /* lbl should be 'Age in years' */

  /* CALL CATS — concatenate items into a character variable */
  a = 'hello';
  b = 'world';
  call cats(result, a, ' ', b);  /* result should be 'helloworld' (CATS strips) */

  /* CALL SCAN — extract nth word from string */
  str = 'the quick brown fox';
  call scan(str, 2, word);  /* word should be 'quick' */

  /* CALL SORTN — sort numeric variables in place (ascending) */
  array nums{3} n1-n3;
  n1 = 3; n2 = 1; n3 = 2;
  call sortn(nums);   /* nums should be 1, 2, 3 */

  /* CALL SORTC — sort character variables in place (ascending) */
  c1 = 'c'; c2 = 'a'; c3 = 'b';
  call sortc(c1, c2, c3);  /* should be 'a', 'b', 'c' */

  /* CALL SYMPUTX — write variable to macro symbol (like SYMPUT) */
  val = 55;
  call symputx('mymacro', val);  /* &mymacro should be 55 after step */

  keep name lbl result word n1 n2 n3 c1 c2 c3;
  output;
run;

title 'M15.6 CALL Routines';
proc print data=work.test;
run;
