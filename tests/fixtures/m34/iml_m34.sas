/* M34.10 : PROC IML — SHAPE, range subscripts, DET, CALL EIGEN.
   Oracles:
     DET({4 3, 6 3}) = 4*3 - 3*6 = -6
     SHAPE({1 2 3 4 5 6}, 2, 3) = {1 2 3, 4 5 6}
     B[1:2, 2:3] on {1 2 3, 4 5 6, 7 8 10} = {2 3, 5 6}
     CALL EIGEN(val,vec,{2 0,0 3}) → val = {3, 2} (descending), vec axis-aligned. */
title 'PROC IML: SHAPE / range subscripts / DET / CALL EIGEN';
proc iml;
  a = {4 3, 6 3};
  d = det(a);
  print d;
  m = shape({1 2 3 4 5 6}, 2, 3);
  print m;
  b = {1 2 3, 4 5 6, 7 8 10};
  sub = b[1:2, 2:3];
  print sub;
  s = {2 0, 0 3};
  call eigen(val, vec, s);
  print val vec;
quit;
title;
