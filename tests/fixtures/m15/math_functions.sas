/* M15.2 : Mathematical functions — CEIL, FLOOR, SIGN, SIN, COS, TAN, */
/* ARSIN, ARCOS, ATAN, ATAN2, SINH, COSH, TANH, FACT, COMB, PERM,      */
/* GAMMA, LGAMMA, DIGAMMA, BETA, ROUNDZ, RANGE, LARGEST, SMALLEST,      */
/* ORDINAL.                                                               */

data work.test;
  /* CEIL, FLOOR, SIGN */
  ceil_2_3 = ceil(2.3);      /* Should be 3 */
  ceil_neg = ceil(-2.3);     /* Should be -2 */
  floor_2_3 = floor(2.3);    /* Should be 2 */
  floor_neg = floor(-2.3);   /* Should be -3 */
  sign_pos = sign(5.0);      /* Should be 1 */
  sign_neg = sign(-5.0);     /* Should be -1 */
  sign_zero = sign(0.0);     /* Should be 0 */

  /* Trigonometric (radians) */
  pi = 3.14159265358979;
  sin_0 = sin(0);            /* Should be ~0 */
  sin_pi_2 = sin(pi/2);      /* Should be ~1 */
  cos_0 = cos(0);            /* Should be 1 */
  cos_pi = cos(pi);          /* Should be ~-1 */
  tan_0 = tan(0);            /* Should be ~0 */

  /* Inverse trig */
  asin_half = arsin(0.5);    /* Should be ~0.524 radians (~30 degrees) */
  acos_half = arcos(0.5);    /* Should be ~1.047 radians (~60 degrees) */
  atan_1 = atan(1.0);        /* Should be ~0.785 (pi/4) */
  atan2_result = atan2(1.0, 1.0);  /* Should be ~0.785 */

  /* Hyperbolic */
  sinh_0 = sinh(0);          /* Should be 0 */
  sinh_1 = sinh(1.0);        /* Should be ~1.175 */
  cosh_0 = cosh(0);          /* Should be 1 */
  cosh_1 = cosh(1.0);        /* Should be ~1.543 */
  tanh_0 = tanh(0);          /* Should be 0 */

  /* Factorial */
  fact_0 = fact(0);          /* Should be 1 */
  fact_5 = fact(5);          /* Should be 120 */
  fact_10 = fact(10);        /* Should be 3628800 */

  /* Combinatorics */
  comb_5_2 = comb(5, 2);     /* Should be 10 */
  comb_10_3 = comb(10, 3);   /* Should be 120 */
  perm_5_2 = perm(5, 2);     /* Should be 20 */
  perm_10_3 = perm(10, 3);   /* Should be 720 */

  /* Gamma functions */
  gamma_3 = gamma(3.0);      /* Should be 2! = 2 */
  gamma_5 = gamma(5.0);      /* Should be 4! = 24 */
  lgamma_3 = lgamma(3.0);    /* Should be log(2) ~= 0.693 */

  /* ROUNDZ (round to zero on ties) */
  roundz_2_5 = roundz(2.5);  /* Should be 2 (toward zero, not 3) */
  roundz_neg_2_5 = roundz(-2.5);  /* Should be -2 */
  roundz_2_7 = roundz(2.7);  /* Should be 3 */

  /* RANGE, LARGEST, SMALLEST */
  range_vals = range(1, 5, 3, 9, 2);   /* max - min = 9 - 1 = 8 */
  largest_1 = largest(1, 1, 5, 3, 9, 2);  /* Should be 9 */
  largest_2 = largest(2, 1, 5, 3, 9, 2);  /* Should be 5 */
  smallest_1 = smallest(1, 1, 5, 3, 9, 2);  /* Should be 1 */
  smallest_2 = smallest(2, 1, 5, 3, 9, 2);  /* Should be 2 */

  /* ORDINAL */
  ord_1 = ordinal(1);        /* Should be "1st" */
  ord_2 = ordinal(2);        /* Should be "2nd" */
  ord_3 = ordinal(3);        /* Should be "3rd" */
  ord_4 = ordinal(4);        /* Should be "4th" */
  ord_11 = ordinal(11);      /* Should be "11th" */
  ord_21 = ordinal(21);      /* Should be "21st" */
  ord_22 = ordinal(22);      /* Should be "22nd" */
  ord_23 = ordinal(23);      /* Should be "23rd" */
  ord_100 = ordinal(100);    /* Should be "100th" */

  output;
run;

title 'M15.2 Mathematical Functions';
proc print data=work.test;
run;
