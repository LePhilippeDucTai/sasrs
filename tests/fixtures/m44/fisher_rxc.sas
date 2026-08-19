/* M44.1 - Fisher's exact test generalized to r x c tables (Freeman-Halton).
   1) Perfect-diagonal 3x3 [[5,0,0],[0,5,0],[0,0,5]] : the most extreme table
      possible for these margins - hand-verifiable without an external SAS
      doc example. P = 1/756756 ; Pr<=P = 6/756756 = 1/126126, matching the
      3! = 6 equiprobable diagonal permutations (the only tables at least as
      extreme as the observed one).
   2) A non-degenerate 2x3 table : exact enumeration path (well under the
      500 000-table guard), same P/Pr<=P statistics.
   3) A 4x4 table (margins 20 each, n=80, built via WEIGHT to avoid 80
      datalines) that exceeds the exact-enumeration guard : falls back to
      the labeled Monte Carlo estimate (fixed seed, 10 000 samples). */

data diag3;
  input row $ col $ w;
  datalines;
A A 5
A B 0
A C 0
B A 0
B B 5
B C 0
C A 0
C B 0
C C 5
;
run;

title 'Fisher exact 3x3, perfect diagonal (most extreme possible table)';
proc freq data=diag3;
  tables row*col / fisher;
  weight w;
run;

data grid2x3;
  input row $ col $;
  datalines;
A X
A X
A Y
A Y
A Y
A Z
A Z
A Z
A Z
B X
B X
B X
B X
B Y
B Y
B Y
B Z
B Z
;
run;

title 'Fisher exact 2x3';
proc freq data=grid2x3;
  tables row*col / fisher;
run;

data grid4x4;
  input row $ col $ w;
  datalines;
A A 8
A B 2
A C 1
A D 1
B A 1
B B 6
B C 2
B D 1
C A 1
C B 1
C C 5
C D 1
D A 1
D B 1
D C 1
D D 4
;
run;

title 'Fisher exact 4x4 (exceeds the exact-enumeration guard -> Monte Carlo estimate)';
proc freq data=grid4x4;
  tables row*col / fisher;
  weight w;
run;
