title 'PROC IML: linear algebra and I/O';

proc iml;
  /* INV */
  a = {4 7, 2 6};
  ai = inv(a);    /* det=10 -> {0.6 -0.7, -0.2 0.4} */
  print ai;

  /* SOLVE : {2 0, 0 3} * x = {6, 9} -> x = {3, 3} */
  b_mat = {2 0, 0 3};
  rhs = {6, 9};
  x = solve(b_mat, rhs);    /* {3, 3} */
  print x;

  /* EIGVAL (symmetric) */
  s = {4 2, 2 1};
  ev = eigval(s);   /* lambda1=5, lambda2=0 descending */
  print ev;

  /* CHOL */
  c_mat = {4 2, 2 3};
  u = chol(c_mat);  /* upper U : {2 1, 0 1.4142} */
  print u;

  /* CALL QR */
  q_in = {1 2, 3 4, 5 6};
  call qr(q_out, r_out, q_in);
  print q_out r_out;

  /* CALL SVDCD */
  sv_in = {1 2, 3 4};
  call svdcd(u_out, d_out, v_out, sv_in);
  print d_out;   /* singular values descending */

  /* I/O: create a dataset and write it */
  mat_out = {1 10, 2 20, 3 30};
  cn = {"id" "val"};
  create work.iml_out from mat_out[colname=cn];
  append from mat_out;
  close work.iml_out;

quit;
title;
