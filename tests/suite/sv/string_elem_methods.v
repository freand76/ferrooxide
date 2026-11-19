// String methods on elements of arrays, queues and dynamic arrays of strings
// (IEEE 1800-2017 6.16: the methods apply to any string operand; `a[i].putc` changes the
// element).
module m;
  string fixed [0:2];
  string q [$];
  string d [];
  int i;
  initial begin
    fixed[0] = "ab"; fixed[1] = "hello"; fixed[2] = "x";
    q.push_back("one"); q.push_back("three");
    d = new[2];
    d[0] = "Zz"; d[1] = "42";
    i = 1;
    if (fixed[i].len() != 5 || fixed[2].len() != 1) $display("FAILED -- fixed len %0d", fixed[i].len());
    if (fixed[1].toupper() != "HELLO" || fixed[i].getc(1) != "e") $display("FAILED -- fixed toupper/getc");
    if (q[i].len() != 5 || q[0].substr(1, 2) != "ne") $display("FAILED -- queue %0d", q[i].len());
    if (d[1].atoi() != 42 || d[0].tolower() != "zz") $display("FAILED -- darray");
    fixed[i].putc(0, "j");
    q[0].putc(2, "Y");
    if (fixed[1] != "jello" || q[0] != "onY") $display("FAILED -- putc %s %s", fixed[1], q[0]);
    foreach (q[k]) if (q[k].len() < 3) $display("FAILED -- foreach %0d", k);
    $display("PASSED");
  end
endmodule
