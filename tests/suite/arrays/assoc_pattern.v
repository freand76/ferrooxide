// An assignment pattern for an associative array (IEEE 1800-2017 7.9.11) gives its
// entries as `key: value` and, with `default: value`, the value read for keys without an
// entry (which does not create the entry); copies keep the default.
module t;
  localparam int K = 7;
  string words [int] = '{default: "hello"};
  int m [string];
  int n [int];
  byte b [int];
  int fails = 0;
  task check(input bit c, input int line);
    if (!c) begin
      $display("FAILED line %0d", line);
      fails++;
    end
  endtask
  initial begin
    check(words[1] == "hello" && words.num() == 0, `__LINE__);
    words[1] = "world";
    check(words[0] == "hello" && words[1] == "world" && words.num() == 1, `__LINE__);
    m = '{"one": 1, "two": 2, default: -1};
    check(m.num() == 2 && m["one"] == 1 && m["two"] == 2 && m["three"] == -1, `__LINE__);
    check(!m.exists("three"), `__LINE__);
    n = '{K: 70, 2 + 1: 30};
    check(n.num() == 2 && n[7] == 70 && n[3] == 30 && n[5] == 0, `__LINE__);
    n = m.num() > 1 ? '{1: 10, default: 5} : '{default: 6};
    check(n.num() == 1 && n[1] == 10 && n[2] == 5, `__LINE__);
    b = '{default: 8'hff};
    n.delete();
    check(n[4] == 5 && b[100] == -1, `__LINE__);
    m.delete();
    m = '{};
    check(m.num() == 0, `__LINE__);
    if (fails == 0) $display("PASSED");
  end
endmodule
