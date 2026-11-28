// A typedef of an associative array of associative arrays, or a queue of queues (IEEE
// 1800-2017 7.8, 7.10): variables, class properties, elements of an associative array of
// the type; packed array keys.
module test;
  typedef bit data_t;
  parameter k = 6, j = 5, l = 2;
  typedef data_t my_ar_t [bit[31:0][k:0]][bit[j:0][l:0]];
  typedef int m_t [string][int];
  typedef int qq_t [$][$];
  class C; m_t p; endclass
  my_ar_t v;
  m_t w;
  qq_t q;
  m_t arr [string];
  initial begin
    C c = new;
    v[3][4] = 1;
    c.p["a"][1] = 7;
    w["x"][2] = 9;
    q.push_back('{1, 2});
    arr["k"]["y"][3] = 4;
    if (v.num() == 1 && v[3][4] == 1 && c.p["a"][1] == 7 && c.p.num() == 1 && w["x"][2] == 9
        && q[0][1] == 2 && arr["k"]["y"][3] == 4 && arr["k"].num() == 1)
      $display("PASSED");
    else
      $display("FAILED");
  end
endmodule
