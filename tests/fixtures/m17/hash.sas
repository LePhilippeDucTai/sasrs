/* M17.2 : objets hash — add/find/check/replace/remove/clear/output/num_items,
   chargement dataset:, multidata, ordered:, et itérateur HITER. */
libname d 'data';

/* 1) Lookup : charge d.class (key=name, data=age) puis enrichit une table. */
data lookup;
  length name $8;
  if _n_ = 1 then do;
    declare hash h(dataset:'d.class');
    h.defineKey('name');
    h.defineData('age');
    h.defineDone();
  end;
  /* find par nom : copie age depuis le hash. */
  name = 'Mary'; age = .; rc = h.find();
  found = (rc = 0);
  output;
  name = 'Nobody'; age = .; rc = h.find();
  found = (rc = 0);
  output;
  stop;
run;

title 'Hash lookup loaded from d.class';
proc print data=lookup noobs;
  var name age rc found;
run;

/* 2) Construction manuelle + add/replace/remove/num_items + output ordered. */
data _null_;
  declare hash scores(ordered:'ascending');
  scores.defineKey('id');
  scores.defineData('pts');
  scores.defineDone();
  id = 3; pts = 30; scores.add();
  id = 1; pts = 10; scores.add();
  id = 2; pts = 20; scores.add();
  /* replace remplace, remove supprime. */
  id = 2; pts = 99; scores.replace();
  id = 3; rc = scores.remove();
  n = scores.num_items;
  put 'num_items after remove=' n;
  scores.output(dataset:'work.scores_out');
  stop;
run;

title 'Hash output, ordered ascending (id 3 removed, id 2 replaced)';
proc print data=scores_out noobs;
  var id pts;
run;

/* 3) Itérateur HITER : parcours avant (ordre ascendant). */
data iterated;
  declare hash g(ordered:'descending');
  declare hiter gi('g');
  g.defineKey('k');
  g.defineData('v');
  g.defineDone();
  k = 1; v = 100; g.add();
  k = 2; v = 200; g.add();
  k = 3; v = 300; g.add();
  rc = gi.first();
  do while (rc = 0);
    output;
    rc = gi.next();
  end;
  stop;
  keep k v;
run;

title 'HITER descending traversal';
proc print data=iterated noobs;
  var k v;
run;
