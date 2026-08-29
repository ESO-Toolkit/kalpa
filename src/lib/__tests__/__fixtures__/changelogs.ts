// Real ESOUI changelogs, captured from
// `https://api.mmoui.com/v4/game/ESO/filedetails/<id>.json` and put through the
// same cleaning `src-tauri/src/esoui.rs` applies before the frontend sees them
// (HTML entities decoded, `[*]` -> `• `, BBCode and HTML tags stripped,
// trimmed). Long ones are truncated; nothing else is edited.

/** esoui.com id 695 */
export const awesomeGuildStore = `version 1.7.8:

Changes:


• Fixed errors in XBox Play Anywhere edition

• Updated for Seasons of the Worm Cult Part 2


version 1.7.7:

Changes:


• Fixed performance issue when swapping between tabs in the guild store UI often

• Updated item style filter data


version 1.7.6:

Changes:


• Added notice text to mails while the guild history is unavailable


version 1.7.5:

Changes:


• Fixed unknown writs filter in "Consumables > Misc" category and changed it to show both surveys and writs


version 1.7.4:

Changes:


• Added unknown writs to "Consumables > Misc" category

• Remade all icons to fit the new UI style

• NOTE: feel free to use the old icons to make an addon that replaces the new ones

• Fixed dependency issue on consoles


version 1.7.3:

Changes:


• Added preliminary compatibility for console

• NOTE: This only means it will load and run without throwing errors, but it won't do anything noteworthy, since there is no UI yet

• Updated for Seasons of the Worm Cult Part 1


version 1.7.2:

Changes:


• Fixed price and unit price header placement in search tab

• Fixed companion armor weight filter not getting applied to server requests

• Fixed bows missing from companion weapon type filter

• Fixed kiosk history list showing some invalid empty week entries

• Fixed error when visiting Leyawiin outlaws refuge when language is set to French

• Updated item style filter data

• Updated for Update 45


version 1.7.1:

Changes:


• Added scribing scripts to consumable category on search tab

• Fixed error on search tab

• Updated for Update 43


version 1.7.0:

Changes:


• Re-implemented mail invoice feature

• NOTE: LibId64 and LibHistoire are new required dependencies

• Updated for Gold Road


version 1.6.10:

Changes:


• Fixed error when clicking sort order dropdown on search tab



version 1.6.9:

Changes:


• Fixed text search not working in bank inventory on sell tab screen

• Temporarily disabled mail invoice feature as it is incompatible with the new history api and needs a complete rewrite

• Updated for Scions of Ithelia



version 1.6.8:

Changes:


• Added missing restoration staff button to companion weapon type filter

• Fixed gamepad mode not being restored correctly when it was set to automatic

• Updated for Secrets of the Telvanni


version 1.6.7:

Changes:


• Fixed error in German and French when visiting the trader in Necrom Outlaws Refuge


version 1.6.6:

Changes:


• Added unit price filter to "All" category

• Added type filters for companion equipment

• Fixed multi button filters not showing their current state on initial store visit

• Fixed search result list position not being reset when visiting stores that already have cached results

• Fixed pending item on sell tab not being cleared correctly when switching guilds or closing the store

• Fixed incorrect entries for Vastyr and Necrom Outlaws Refuge in guild store list

• Updated for Necrom


version 1.6.5:

Changes:


• Fixed guild kiosk detection from guild finder not working in some cases

• Fixed rare error when entering a row on the guild trader list while closing the menu


version 1.6.4:

Changes:


• Fixed some input boxes on the search tab not being clickable in certain situations

• Fixed writs with 0 vouchers causing an error when trying to list them

• Updated Russian translations (thanks mychaelo)

• Updated Brazilian translations (thanks mlsevero)

• Added Simplified Chinese translations (thanks AimerAbyss)


version 1.6.3:

Changes:


• Fixed text filter autocompletion race condition

• Fixed text filter no longer applying correctly on the server side after some time

• Switched to weblate for managing translations

• Updated Polish translations (thanks generaluploads)

• Removed LAUNIG as it no longer serves any real purpose


version 1.6.2:

Changes:


• Fixed potential error when moving the mouse over a search result

• Fixed price warning on sell tab not showing its tooltip

• Fixed price selector buttons not working in some cases

• Added junk category to player inventory on sell tab


version 1.6.1:
Patrons received exclusive early access here.

Changes:


• Fixed several issues when the server sends its initial response before the store has finished initialization

• Fixed error when using server side sort orders other than the default one

• Fixed guild owner info getting mixed up when visiting multiple kiosks

• Fixed search list showing on other tabs in some cases

• Updated German translations


version 1.6:
Patron exclusive release

Changes:


• Fixed search results not automatically filtering newly learned motifs (and similar) between store visits

• Fixed missing icons on jewelry trait filter

• Fixed underlying cause for “guildState not correctly initialized” messages in chat

• Updated text filter autocompletion to only show results relevant to the currently selected filter categories

• Updated how server responses are handled

• NOTE: This should fix several cases of activities getting stuck

• Updated how the trading house UI is opened

• NOTE: The UI will now show instantly in a locked state while waiting for the server to respond (no more random waiting times before it shows!)

• Improved performance on first store visit

• Automatically switch to keyboard UI when opening the store in gamepad mode and back when the store is closed

• Updated sell tab to allow listing items from the bank while at a banker NPC

• Updated guild info scene so it allows to go back without closing out of the interaction while at a store

• Updated item style filter for High isle


version 1.5.8:

Changes:


•  Fixed sort headers not being positioned correctly on the search tab


version 1.5.7:

Changes:


•  Added Polish translation (thanks generaluploads)


version 1.5.6:

Changes:


•  Optimized how companion item filter requests data from the server

•  Fixed errors on PTS

•  Updated for Deadlands


version 1.5.5:

Changes:


•  Adjusted filters for outfit style pages and other collectible trophies


version 1.5.4:

Changes:


•  Added option to filter for no traits in equipment categories

•  Added new category and trait filter for companion items

•  Updated Master Merchant integration to use recommended functions

•  Updated item style filter

•  Updated for Blackwood


version 1.5.3:

Changes:


•  Fixed compatibility issue with inventory filter addons


version 1.5.2:

Changes:


•  Fixed furniture preview no longer working after latest game update

•  Fixed mail invoice incorrectly showing that the listing fee is refunded


• NOTE: It never was refunded, sorry for any confusion this may have caused over the years.


•  Fixed bound items not getting filtered in craftbag

•  Fixed craft bag not showing when it is selected and the store is reopened

•  Fixed error when store is closed while not being a member of a guild

•  Fixed some item style filter category names showing translation modifiers in some languages

•  Fixed level range filter not updating input fields when invalid values are entered

•  Updated for Flames of Ambition


version 1.5.1:

Changes:


•  Fixed incorrect iso week number during last week of 2020


• NOTE: Make sure to update LibDateTime!


•  Minor improvement to how the item set collection filter works internally

•  Updated dependencies to require latest version of LibDateTime and LibDebugLogger


version 1.5.0:

Changes:


•  Updated search tab categories to match the new in-game subcategories


• NOTE: Going forward the official subcategories will be used as the source for how the category filter behaves


•  Updated collectible ownership filter to work with the new item set collection feature

•  Updated many filters to use newly added in-game icons and removed the old icon files

•  Updated quality filter icons to render their color dynamically


• NOTE: I’ve left the old pre-rendered icons in for now, but will remove them in a future version


•  Updated item style filter for Markath

•  Updated character limit of the text filter input


• NOTE: You can now store up to ~30k characters, but just because you can doesn’t mean you should, as it will lead to severe performance issues in most cases


•  Updated code to make use of new functions that have been added in the past few game updates

•  Updated log levels to reduce log spam for other developers and regular users


version 1.4.4-beta:

Changes:


•  Fixed locked filters changing their values in some situations (thanks DewiMorgan!)

•  Removed libCommonInventoryFilters dependency


version 1.4.3-beta:

Changes:


•  Fixed the reason why a location entrance cannot be matched in some languages

•  Removed custom sell tab filter in favor of the new ingame filtering


• NOTE: This change only applies on PTS and once Markarth goes live


•  Updated for Markarth


version 1.4.2-beta:

Changes:


•  Fixed error when a location entrance cannot be matched


version 1.4.1-beta:

Changes:


•  Fixed kiosk text matching not working in Russian

•  Removed LibRunebox dependency

•  Removed collectible ownership filter from trophy subcategory


• NOTE: In my tests it didn't apply to any of the items in that category and was just misleading


•  Updated for Stonethorn


version 1.4.0-beta:

Changes:


•  Improved store detection code and save data format


• NOTE: I've tested the changes to the best of my ability, but still recommend you create a backup if you value your store list data


•  Added reusable search tab when searching for specific items

•  Added "Search For Item" context menu entry to craft bag items on sell tab

•  Added map ping when showing a store on the map

•  Added code to clean up incorrect kiosks and stores from data

•  Removed LibStub dependency (and sadly LibCustomTitles as a result)

•  Updated item style filter data

•  Updated French translation (thanks igx31!)

•  Updated for Greymoor

•  Fixed error when the selected guild id is nil when a guild store is opened

•  Fixed resetting a search that is not selected resets the active search too

•  Fixed items not getting marked as sold out when they are no longer available

•  Fixed unit price filter not showing in "Crafting > All" category

•  Fixed regular clothes not showing in the "Apparel > Costumes" category

•  Fixed "one activity queued" status message getting stuck in some cases

•  Fixed time remaining and unit price headers overlapping in the official Russian localization

•  Fixed entrance pin detection not working correctly in the official Russian localization

•  Fixed clicking an entry in the guild trader history not opening the correct guild list entry

•  Fixed some stores not getting detected on characters that haven't visited certain locations yet


• NOTE: It should now be able to locate all existing kiosks even on new characters. Let me know if that's not the case for you!


•  Fixed "Open On Map" not always showing the correct map

•  Fixed several bugs that allowed NPCs and locations that aren't actually kiosks to get detected as such

•  Fixed owner data not getting updated in some cases for guilds you are a member of


version 1.3.1-beta:

Changes:


•  Fixed error when startup integrity check failed

•  Updated localization files

•  Added Spanish localization (thanks Inval1d!)

•  Added id for the Arkadius Trade Tools deal filter to the list


version 1.3.0-beta:

Changes:


•  Changed "Show Details" in kiosk list context menu to switch to the guild list for the current owner

•  Added "Show Details" and "Open Guild Info" keybinds to kiosk and guild list

•  Updated item style filter data

•  Fixed text filter not getting applied to server side search in most cases

•  Removed calls to LibStub and dropped support for LibFilters2

•  Switched to LibChatMessage for showing messages in chat


• NOTE: Make sure to install LibChatMessage v1.1 if you haven't done so yet!

• NOTE: The setting for short chat message prefix has been dropped in favor of LibChatMessage's own setting. Use "/chatmessage tag short" to enable it.


•  Updated for Harrowstorm


version 1.2.3-beta:

Known issues:


• When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

• New texts have not been translated to French and Russian yet.

• Items that have already disappeared from a store will show up locally until you log out or reload the ui.


Changes:


•  Improved guild info detection for guild list with new api functions and fixed various bugs

•  Fixed error when sorting the guild list by owner

•  Fixed error when pressing "Load Details" button in mails

•  Updated for Dragonhold


version 1.2.2-beta:

Known issues:


• When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

• New texts have not been translated to French and Russian yet.

• Items that have already disappeared from a store will show up locally until you log out or reload the ui.


Changes:


•  Fixed error when no more guilds are available for trading while the store interface is open

•  Fixed error when other addons try to retrieve item data while no guild is selected

•  Updated Japanese translation (thanks marify)

•  Updated Brazilian translation (thanks mlsevero)

•  Updated LibCustomTitles

•  Added dependency version requirements to manifest

•  Updated for Scalebreaker


Can you believe it's been five years already?

version 1.2.1-beta:

Known issues:


• When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

• New texts have not been translated to all languages yet.

• Items that have already disappeared from a store will show up locally until you log out or reload the ui.


Changes:


• Fixed error on login when guild lost their trader


version 1.2-beta:

Known issues:


• When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

• New texts have not been translated to all languages yet.

• Items that have already disappeared from a store will show up locally until you log out or reload the ui.


Changes:


• Fixed furniture item preview


• NOTE: Huge thanks to ZOS for adding the new API required for this!


• Fixed tooltip related compatibility issues with other addons due to a change made in 1.1

• Fixed guild selector showing guilds in the wrong order

• Fixed last selected guild getting lost when visiting a kiosk

• Fixed hired guild tooltip staying visible in some cases

• Fixed listing tab being enabled for guild kiosk you are not a member of

• Fixed incorrect guild owner information getting stored during lag when visiting more than one store

• Fixed several bugs that can lead to wrong information getting stored when the guild system is disabled

• Fixed guild list showing a guild that already has a confirmed kiosk as potential owner for the one they had in the previous week

• Renamed "Consumable > Ava Repair Kits" to "Consumable > Tools" and moved repair kits there


• NOTE: This change only affects the search tab. The filtering on the sell tab will see a complete overhaul in a future update


• Updated code to use the new global guild ids instead of guild names where applicable


• NOTE: As a result the ITEM_DATABASE_UPDATE callback now passes the guildId instead of the guildName


• Added guild finder as data source for the guild list


• NOTE: This has not been tested in other languages besides English yet. Let me know if you encounter any problems


• Added button in the store UI to open the guild finder info page for the current guild

• Added buttons to open the guild finder info page to the guild list detail page and context menu

• Added setting to specify which tab the store should be opened on when visiting it from a banker

• Added library version check. This will show a detailed message when the addon cannot be loaded due to an incompatible version of a library being used. 


version 1.1-beta:

Known issues:


• Item preview does not work for cached items. If you want to preview something, use the new "Search For Item" context menu entry to request that specific item again.

• When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

• New texts are not final and have not been translated to other languages yet.

• Items that have already disappeared from a store will show up locally until you log out or reload the ui.


Changes:


• Fixed purchasing a guild tabard not working as intended

• Fixed level range filter for Orichalcum and Quicksilver Ingots

• Fixed text filter auto completion not always showing all results


• NOTE: It will now show all items, even when they are not part of the currently selected category. This will be corrected in a future update.


• Fixed an issue that caused some addons to show their tooltip twice


• NOTE: please let me know if there are still addons affected by this


• Automatically open guild store on the sell tab when at a banker

• Added filter for unowned collectibles

• Added support for selling and searching trash quality items

• Added feedback and donate button to settings panel

• Added donate button to store footer

• Improved layout of the activity window

• Removed all bundled standalone libraries


• NOTE: you have to install the following libraries in order to use AwesomeGuildStore:


• 

• LibCustomMenu

• LibDebugLogger

• LibDateTime

• LibAddonMenu-2.0

• LibMapPing

• LibGPS

• LibPromises

• LibTextFilter

• LibGetText

• libCommonInventoryFilters

• LibRunebox



• Updated for Elsweyr


version 1.0.2-beta:

Before using this version, please make a backup of your saved variables, just in case something goes wrong.

Known issues:


•  Item preview does not work for cached items. If you want to preview something, use the new "Search For Item" context menu entry to request that specific item again.

•  Some addons are currently not fully compatible. Here is a list of the ones that have been reported so far:


•  Postmaster (unconfirmed)

•  NoAccidentalStealing (unconfirmed)

•  Autocategory (unconfirmed)

•  BankManager Revived (unconfirmed)

•  PriceTracker Updated (unconfirmed)

•  ESO Master Recipe List Alt Format (unconfirmed)

•  Set Tracker (unconfirmed)


•  When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

•  New texts are not final and have not been translated to other languages yet.

•  Items that have already disappeared from a store will show up locally until you log out or reload the ui.



Changes:


•  added text when no search results are available or request is running

•  added keybinds 1 - 5 to quickly switch between your guilds at a banker

•  improved multi button filter UX


• NOTE: you can now press and hold any combination of the ctrl and shift key to interact with multiple buttons


•  improved addon integrity check to show a proper chat message instead of throwing an assertion error on load

•  improved listing requests to only queue when needed

•  improved footer text visibility and added a highlight animation when hovering over it

•  fixed multiple selected filter buttons not getting applied to the server side search correctly

•  fixed an error when trying to confirm an item purchase after switching guilds

•  fixed cancel item chat notification showing even when it is disabled in the settings

•  fixed pending item listing not getting cleaned up correctly when the store is closed

•  fixed last selected guild name showing on a kiosk after using the store at a banker before

•  fixed list item action hanging forever in some rare cases

•  fixed a rare error when visiting the guild store for the first time during a session

•  updated bundled LibDebugLogger


version 1.0.1-beta:

Before using this version, please make a backup of your saved variables, just in case something goes wrong.

Known issues:


•  Item preview does not work for cached items. If you want to preview something, use the new "Search For Item" context menu entry to request that specific item again.

•  Some addons are currently not fully compatible. Here is a list of the ones that have been reported so far:


•  Postmaster (unconfirmed)

•  NoAccidentalStealing (unconfirmed)

•  Autocategory (unconfirmed)

•  BankManager Revived (unconfirmed)

•  PriceTracker Updated (unconfirmed)

•  ESO Master Recipe List Alt Format (unconfirmed)

•  Set Tracker (unconfirmed)


•  When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way. This is actually a game bug (happens without AGS too), so it is up to ZOS to fix it.

•  New texts are not final and have not been translated to other languages yet.

•  Items that have already disappeared from a store will show up locally until you log out or reload the ui.


Changes:


•  added queue and execution time display to the activity log

•  fixed activity log showing incorrect times and durations

•  fixed item link not showing in the activity log in many cases when listing items

•  fixed sell price display changing to an incorrect value when listing items

•  fixed listing count not updating in some cases

•  fixed listings sort order not being applied in many situations

•  fixed cancel button showing again once listings are refreshed when cancelling multiple items


version 1.0.0-beta:

This is the first beta release of AwesomeGuildStore 1.0. A huge thank you to everyone who tested the development builds this past week, I finally feel it is stable enough to share with a wider audience. This doesn't mean that everything is finished or perfect yet and I will continue to upload new versions as needed, but I also don't want to keep you waiting any longer.

Before using this version, please make a backup of your saved variables, just in case something goes wrong.

Here is a list of known issues:



•  Item preview does not work for cached items. If you want to preview something, use the new "Search For Item" context menu entry to request that specific item again.

•  Some addons are currently not fully compatible. Here is a list of the ones that have been reported so far:


•  Arkadius' Trade Tools -> can lead to incorrect data getting stored, so do not use them together until the problem is resolved

•  PerfectPixel (unconfirmed)

•  Postmaster (unconfirmed)

•  NoAccidentalStealing (unconfirmed)

•  Autocategory (unconfirmed)

•  BankManager Revived (unconfirmed)

•  PriceTracker Updated (unconfirmed)

•  ESO Master Recipe List Alt Format (unconfirmed)


•  When using the new multi stack listing feature on the sell tab, the price display changes for subsequent stacks. This is purely cosmetic and the items are listed at the correct price.

•  When canceling multiple items on the listings tab, the cancel button shows up again when the list is refreshed.

•  When closing the trading house via the gamepad ui toggle keybind, AGS won't do anything on the next store visit. You need to close it manually for now. Closing it manually will also get you unstuck in case you accidentally closed it that way.

•  New texts are not final and have not been translated to other languages yet.

•  Items that have already disappeared from a store will show up locally until you log out or reload the ui.



And the actual changes:


• Switched to using semantic versioning and continuous build numbers for the addon version

• Completely rewrote the filter system from scratch


• This was long overdue and it is now a lot more flexible, faster and allows other addons to add filters more easily


• Implemented a new "search tabs" system, which allows to have multiple searches open in parallel (like browser tabs)


• This replaces the old search library and favorites (the new search history is not yet implemented, but will follow soon)

• Due to the changes in the reworked filter system, it is unfortunately not possible to bring the old favorites over into the new system. you'll have to set up your searches from scratch. Sorry for the inconvenience.

• Right click menu with many options. You can change the order they appear in, rename them, reset the label or the filters, duplicate them, lock them to prevent changing filters or delete them entirely


• Finished the activity system (took only three and a half years)


• All server requests are now handled by this system, so you can use the store without having to wait for answers from the server.

• Added an activity panel to view and control what is happening in the background (it can be accessed by clicking on the status text on the bottom of the store)


• Cache search results locally and automatically handle searching in background


• All the filtering now happens locally and instantly while the addon automatically requests more results as needed

• Newly listed items will be searched in regular intervals, but you can also double click the active search in order to trigger a refresh


• Implemented local sorting of results (e.g. by item quality)

• Replaced recipe improvement filter with a new recipe crafting knowledge which hides recipes you cannot craft yet

• Added furniture category filter

• Added master writ craft type and writ voucher range filters

• Improved level filter to be available in all categories where it makes sense

• Split trophy category into consumable and misc trophies

• Improved filter input UX

• Added new slider to control how many listings you will create when selling something (e.g. sell 5 stacks of 100 leather for 1000g per stack)

• Improved drag and drop handling on sell tab

• Removed obsolete settings

• Added LibDebugLogger


• This small library will log errors and debug output into the saved variables

• Per default it will only log basic information, but it can be configured to log everything including stacktraces which can be very important when debugging problems.


• Added footer with version information to the store

• Implemented a new developer API


• Check API.lua and Callbacks.lua for details

• If you need to access something that is not specified in those files, please let me know so I can add it to the API. Everything else is for internal use only and subject to change at any time.


• Fixed many old bugs

• Introduced even more new bugs

• Kept some of the old ones just in case


version 0.43.5:


• fixed error when trying to filter for costumes

• removed the workaround to fix insecure code errors (ZOS fixed this in the basegame)

• added code to prevent errors when the Wrathstone DLC goes live (AGS v0 won't be compatible with the game anymore and you'll have to upgrade to v1 once it is released)


version 0.43.4:


• marked LibCustomMenu and LibStub as dependencies (they come bundled with AGS, but you need to make sure you enabled them in your addon menu!)


version 0.43.3:


• updated LibCustomMenu (it's no longer loaded via AwesomeGuildStore, so make sure you enable it in your addon menu!)

• fixed level and quality filter not working correctly in some cases


version 0.43.2:


• updated libraries

• updated for Murkmire


version 0.43.1:


• changed formatting of currency values to use game internal function

• updated filter preset for furniture

• updated libraries

• updated for Wolfhunter


version 0.43:


• added drag and drop support for listing items from the craft bag

• added integrity check at startup
	
	
• NOTE: this will produce an assertion error on load if files are missing and the addon won't continue loading. That way it will be easier for me to tell which errors are due to missing files and which errors are real bugs
	

• fixed insecure code errors when using inventory items via the keybind after opening the craft bag on sell tab before opening the inventory (thanks votan and shinni)

• fixed "add to listing" context menu showing on right clicking the listing slot on the craft bag sell tab


version 0.42.2:


• added jewelry materials button to furnishing material type subfilter


version 0.42.1:


• added jewelry sketches button to recipe type subfilter

• updated LibFilters


version 0.42:


• added new category for jewelry, following the example of the ingame UI

• removed subcategory for jewelry from the apparel category
	
	
• NOTE: old entries in the search library will still show "Apparel > Jewelry". When you select them, a new entry will be created and you can remove the old one, once you no longer need it
	

• added shields to "Apparel > All"

• added new jewelry traits to subfilter

• added new combined subcategory for all trait materials

• added new subfilter for trait materials
	
	
• NOTE: like with the jewelry entries in the search library, the old ones will still show. When you select them, a new entry will be created and you can remove the old one, once you no longer need it
	

• added new subcategory and subfilter for jewelry materials

• removed trait items from "Crafting > All"
	
	
• NOTE: This was unfortunately necessary due the same old API limitations that prevented that whole subcategory in the past
	

• updated item style filter for new styles

• fixed item style filter loading some styles incorrectly (e.g. Hlaalu as Dark Elf + Nord)

• fixed item price overlapping the research icon

• added id for the WritWorthy Voucher Price filter to the list

• updated bundled libraries

• add Brazilian localization (thanks mlsevero!)

• updated for Summerset


version 0.41.1:


• updated LibCustomMenu (fixes retrieve menu sometimes showing when trying to list items from the crafting bag - thanks votan)

• use LibCustomMenu for registering context menu entries to ensure future compatibility

• updated Russian localization (thanks Kirix)


version 0.41:


• improved timing for the auto search when opening the guild store

• fixed an error when opening guild store before the inventory and afterwards trying to interact with items in the inventory via keybinds

• fixed errors for the German localization when visiting certain traders while the guild list feature is active (thanks snow)

• updated LibCustomMenu and libCommonInventoryFilters (fixes an error on the sell tab when ingame filters are used instead of the AGS filters - thanks votan)

• updated French localization (thanks lexo1000)

• updated for Dragon Bones


version 0.40:


• added new setting for short chat message prefix ( instead of )

• added "set to character level" button to level filter on search tab

• fixed page item count showing an incorrect number in some situations

• fixed known motif filter not working for Ebonshadow chapters

• fixed error when visiting the Orsinium Outlaws Trader in French with Guild Store List active (thanks Ayantir)

• fixed an issue that could cause secure context violations (thanks Votan)

• updated German translation

• updated libCommonInventoryFilters, LibMapPing and LibGPS


version 0.39.1:


• updated libCommonInventoryFilters (fixes remaining missing searchboxes)


version 0.39:


• added trading guild tab to the guild list feature (shows all visited guilds and their kiosk history)

• fixed sell tab category buttons overlapping with the ones from the stock UI

• fixed style filter showing unlabeled entries in the "new" category

• updated libCommonInventoryFilters (fixes missing search boxes)


version 0.38.3:


• fixed search library showing up on next game start when it was toggled off


version 0.38.2:


• fixed purchase message not showing the guild name when buying at a kiosk

• fixed some potential errors when savedata got corrupted

• implemented savedata autorepair functionality which will set missing values on load


version 0.38.1:


• fixed compatibility with MM deal percentage

• updated Russian localization (thanks Kirix)


version 0.38:


• added settings to revert new behavior of the stock UI (minimize chat, reset filters on exit)

• added warning when sell price is below vendor price

• colorized quality names in search history tooltip

• fixed search library not properly hiding when the guild store is not left in an orderly fashion

• prevent search result list jumping to top when an item is purchased

• keep purchased items in result list until page is left (can be switched off in settings)

• updated French localization (thanks Ayantir)

• updated bundled libraries

• prepared for Clockwork City update


version 0.37.6:


• updated item style filter to correctly show style names in non-english clients

• fixed an error on startup when the kiosk in Vivec City Outlaws Refuge has not been visited before with the Guild Trader List active


version 0.37.5:


• updated item style filter to use checkboxes in the context menu

• fixed an error when visiting the kiosk in Vivec City Outlaws Refuge with the Guild Trader List active

• updated libraries


version 0.37.4:


• fixed an error when opening the guild store introduced in 0.37.3

• updated filter for provisioning furnishing materials
	
	
• NOTE: only newly listed items will show up in the correct category. Items listed before the update will continue to show in provisioning > rare materials so you should relist your decorative wax!
	


version 0.37.3:


• rewrote item style filter from scratch

• updated libraries

• updated for Horns of the Reach


version 0.37.2:


•  updated Russian localization

•  updated libraries


version 0.37.1:


•  updated French and German localization

•  added level filter to consumable subcategories which can support it now

•  fixed level filter not being removed correctly when switching to a category without it


version 0.37:


•  switched to a new localization system

•  added new filter options for drinks, provisioning ingredients, siege items and furniture crafting materials

•  fixed perfect roe not showing up in provisioning ingredients

•  partially fixed furnishing material subcategory
	
	
• NOTE: Decorative Wax is currently found in Provisioning Ingredients / Rare Ingredients due to how ZOS has changed the item categories. This will be fixed in a future game update
	

•  partially fixed furnishing subcategory
	
	
• NOTE: crafting stations, light and target dummies should now work as expected
	

•  added fish to consumable > all 
	
	
• NOTE: it has been in consumable > container for a long time already
	

•  added collectibles to misc > trophy and a new button for rare fish to the trophy subfilter 
	
	
• NOTE: some event related collectibles can be sold on the guild store
	

•  increased active subfilter button limit to 24 from 8 
	
	
• NOTE: currently only affects the trait and enchantment filters for equipment
	

•  removed duplicate crafting > all button on sell tab 
	
	
• NOTE: this also fixes the furnishing material button being off screen
	

•  fixed many issues with guild trader list in languages besides English

•  fixed some occurrences of "Warning: Could not match kiosk name"

•  fixed a rare problem where looking at the vivec city outlaw's refuge trader would throw an error

•  updated bundled libraries


version 0.36:


• added "All Materials" button to crafting category on search tab

• fixed armor trait filter not working for intricate shields

• updated for Morrowind


version 0.35.2:


• updated Russian translation (thanks KiriX)

• updated French translation (thanks Ayantir)

• updated German translation


version 0.35.1:


• fixed listing of sub stack quantities not always working (thanks everyone who reported it - let me know if you still have issues after this fix)

• fixed a nil error when trying to open the guild store after switching language while the kiosk list is enabled

• updated bundled libraries


version 0.35:


• improved output when selling from craft bag or partial stacks fails

• implemented unit price support for master writ vouchers

• moved furnishing materials into their own category

• added new ingredient types to alchemy material filter

• added new food type filter to food category

• fixed pending items sometimes not getting cleared when they are sold

• fixed position of the master writ category button on the sell tab

• removed compatibility code


version 0.34.2:


• fixed a bug which could cause full stacks being sold instead of the chosen quantity in some cases

• changed how pending items are cleared on the sell tab in order to improve compatibility with other addons

• changed how the trader dialog is skipped in an attempt to fix occasional long waiting times when the "Skip guild kiosk dialog" option is on

• changed how resetting search tab filters works in order to account for a newly added base game feature.
	
	
• NOTE: as a result the "Remember filters between store visits" setting has been removed and is now permanently on
	

• changed how filters are sorted - they will now always appear in the same order (server side > local > 3rd party)

• removed level filter for categories that do not support it

• added filter categories for master writs and furniture items

• added new filter options for recipes and trophies
	
	
• NOTE: the new furniture recipes are found in the consumable > recipe category
	
• NOTE: the recipe improvement filter is currently not fully compatible with the new recipes
	

• improved performance when opening the trading house for the first time and when switching between item categories

• increased width of slider input boxes on sell tab so the whole unit price is visible for more expensive items

• added a new tab to the guild menu showing a list of all guild traders in Tamriel
	
	
• NOTE: this feature is currently off by default and you need to activate it in the settings first
	
• NOTE: the list is updated whenever you visit a trader as there is no API to get global trader data
	
• NOTE: trader names are language specific and when you switch the client language, the tab stays disabled
	
• NOTE: you can delete all data in order to start over in a different language via the settings menu
	
• NOTE: it hasn't been tested in any language besides English - please report any errors you may encounter
	

• updated bundled libraries

• updated API version

• updated French translation (thanks Ayantir)

• updated German translation


version 0.33.2:


• fixed BoP tradeables showing up on sell tab

• updated libraries

• updated api version


version 0.33.1:


• fixed level filter text input not working as expected (#1806)

• updated Russian translation (thanks KiriX)

• updated Japanese translation (thanks k0ta0uchi)


version 0.33:


• updated item style filter with new styles

• added overall price of all listed items for a guild on the listings tab

• added a message when all items on a page are hidden by local filters

• changed how 3rd party filters are marked

• fixed some filters being marked as local even though they are server side

• fixed level filter not saving its state correctly in some cases
	
	
• NOTE: If you have favorites that have been affected by this bug, you may need to update them
	

• updated LibFilters to 2.0r2, LibCustomTitles to r12 and LibAddonMenu to r22

• updated Russian translation (thanks KiriX)

• updated French translation (thanks Ayantir)

• updated German translation

• removed compatibility code


version 0.32.5:


• fixed an error caused by a change in 0.32.4 (thanks for reporting it estera)


version 0.32.4:


• fixed selling of stacks smaller than the last sold amount not working

• fixed input of decimal values not working properly

• fixed quantity input accepting decimals

• fixed last sold values not being saved properly for potions and poisons


version 0.32.3:


• updated LAM to r21
	
	
• NOTE: This should fix the error in line 50 of SellTabWrapper.lua that some of you have been seeing. If it still shows up, please let me know
	

• fixed keybind option not behaving as it should when setting it to something different than the default


version 0.32.2:


• updated LAM to fix a widespread error (Thanks to everyone who reported this and special thanks to Randactyl for testing!)

• updated manifest. This version of AGS is compatible with PTS and live. If you find any errors on the PTS, please report them


version 0.32.1:


• added Japanese localization (thanks to k0ta0uchi)

• fixed upgrade procedure for very old save data


version 0.32:


• implemented craft bag support for sell tab
	
	
• You can switch between the inventory and the craft bag with the buttons in the top left corner of the sell tab
	

• added quantity and price per unit slider on sell tab
	
	
• The sliders will remember the quantity and unit price when you sell an item
	
• The quantity slider offers buttons to quickly select the last sold quantity or the full stack. The last selected quantity will be automatically selected if available or the stack size otherwise
	
• The unit price slider offers buttons to quickly select the default value (3 * item value), the last sold unit price and Master Merchant's average price. Depending on availability it will pick the last sold price, Master Merchant price or the default price in that order. It will also use Master Merchant's last sold price if no data is saved in AGS
	
• NOTE: You need one free inventory slot to sell anything that is smaller than the stack or coming from the crafting bag, as it will be moved there and then sold
	
• NOTE: you should also disable Master Merchant's price calculator as it is no longer required and will overlap with other UI elements
	

• fixed items not getting fully deselected when changing guild on sell tab

• fixed keybind resetting to default

• added LibCustomMenu for the context menu in the search library

• updated libCustomInventoryFilters and LibCustomTitles

• removed compatibility code and fully updated for Dark Brotherhood


version 0.31.1:


• first compatibility pass for DB on PTS
	
	
• NOTE: This version is compatible with live and pts
	

• added poison item types
	
	
• NOTE: Due to the new 9th item type in the consumable category, fish will no longer be part of "Consumable > All". You can still find them in "Consumable > Container".
	

• added new item styles

• adjusted filters for removal of veteran rank

• replaced some icons with new ingame icons


version 0.31:


• made "disable local filters" key re-assignable

• implemented advanced page navigation

• fixed skip empty pages not starting over when a filter was changed

• fixed undesired results showing up when clicking a favorite or history entry with auto search enabled

• fixed buttons of other addons not getting relocated properly in some cases (thanks uladz)

• updated French and Russian translations (thanks to Ayantir and KiriX)

• added LibCustomTitles


version 0.30.2:


• fixed an error when visiting a guild store while on the gamepad UI

• update LibAddonMenu to r20


version 0.30.1:


• fixed starting a different search while skipping empty pages is not going back to page 1

• fixed skip empty pages continuing to queue searches even when not on the search tab


version 0.30:


• added sort headers for favorites (sort by name or search count) in search library

• fixed the search library window going completely off screen in some rare cases

• ctrl key temporarily disables local filters in search tab

• added option to automatically skip empty pages when local filters hide all results (off by default)
	
	
• NOTE: Master Merchant has a similar option. It is highly recommended that you only activate one of them at a time!
	

• automatically remove focus from text fields when changing the selected guild via mouse wheel

French translations are provided by Ayantir (Thanks!)

version 0.29.1:


• updated LibFilters to r16.1

• updated LibAddonMenu to r19


version 0.29:


• fixed several bugs and issues with the new text filter which may cause some patterns to behave differently in the new version
	
	
• the '^' and '-' operators have been change to behave more intuitively (thanks to Godzynon for reporting this)
	
• '!' continues to behave as before and can be used as a replacement if your filter pattern relies on it
	

• fixed the known trait filter not working for all item types (thanks Wandamey)

• added buttons for all new craftable styles to the style filter


version 0.28:


• added recipe improvement filter
	
	
• NOTE: if the new icons do not show up, either restart the game or delete your shader cache and restart it
	

• fixed ancient orc motif not being filtered by the known motif filter

• improved unit price filter to support prices below 1 and decimals

• implemented some improvements for addon developers suggested by merlight
	
	
• instead of using the global CALLBACK_MANAGER, AGS now uses its own callback object. See StartUp.lua for details
		
		
• NOTE: the old way via CALLBACK_MANAGER is still in place, but will be removed in a future version
		
	
• added compatibility for custom tabs in the guild store
		
		
• NOTE: AGS can call the RunInitialSetup, OnOpen, OnClose methods for your custom tab if you register it inside the BeforeInitialSetup callback. See TradingHouseWrapper.lua for details
		
	
• simplified how local filters are applied
	

• improved text filter to allow exclusion of terms besides some other things. see addon description for details. (thanks for the help merlight!)


version 0.27.4:


• updated libFilters to r16

• fixed mercenary motifs not getting filtered

• slash command (/ags reset) for resetting history window size & position


version 0.27.3:


• fixed item links in purchase, sell and cancel messages showing as []


version 0.27.2:


• updated to API version 100013

• added support for FastAPI


• NOTE: FastAPI is a small program that automatically changes the API version of supported addons so they don't show as out of date in game when a major update occurs


• added a tiny delay before local filters (e.g. text filter) start filtering so they do their work less often while you change them


version 0.27.1:


• added a new callback for external filter registration to fix their state not being restored when auto search is active (e.g. MasterMerchant's Deal Filter)


version 0.27:


• fixed malachite shards not showing in the materials > all category on the sales tab

• fixed local filters not working on first store visit in some cases

• added item style, item set and crafted item filters

• added notifications for when an item listing is cancelled

• added notifications for when an item listing is created


• NOTE: This is disabled by default in the settings as MM has a similar feature that cannot be disabled at the moment.
You will need to delete line 288 and 289 in MasterMerchant.lua if you want to use both addons and use the AGS message instead


• extended queue system to support page buttons

• implemented highlighting of current filter state in search library

• changed colors of show more and show previous page entries to be easier distinguishable

• updated French, Russian (thanks Ayantir and KiriX) and German Localization


version 0.26:


• updated API version to 100012

• updated libFilters to r15.2

• updated level filter to support vr16

• fixed sell tab sometimes not showing as enabled on first store visit

• updated Russian localization (thanks to KiriX)


version 0.25


• added fish to consumable > container and consumable > all

• added LibCustomFilters and an option to disable sell tab filters in favor of regular ingame inventory filters

• updated LibStub to r4 and LibFilters to r15.1

• added option to automatically skip the dialog at a guild kiosk


• NOTE: You can hold the shift key while talking to a trader to show the usual dialog


• added shift+click to cancel listings without showing the confirmation dialog

• implemented first part of revised API cooldown handling


• For now only applies to search, cancel listing and request listing operations

• This means the search button will no longer be greyed and when you press it, it will queue a search request which will be executed once the store is ready

• Only one search request may be queued at a time

• The auto search feature will also use this queue and now works at the bank guild stores too


• Updated French localization (thanks to Ayantir)


version 0.24


•  removed shields from "Weapon > All"

•  added chat purchase notification

•  fixed error on sales log refresh after joining or leaving a guild

•  updated French and Russian localization (thanks to Ayantir and KiriX)


version 0.23


•  added option to disable invoice in mails while still showing sales information

• Note: The invoice is now disabled by default and you have to reactivate it if you want to see it

•  fixed confusion about profit and sent gold on the invoice (thanks QuadroTony and Ayantir)

•  improved compatibility with some mail addons

• Note: The invoice and show more button are currently not working when MailR is also active. This requires some changes in MailR which I already asked the author to add

•  added new filters for unit price, researchable traits and unknown runes

•  added experimental auto search feature

• Note: For technical reasons this is currently not working when opening the guild store at a bank and also causes strange behavior when favorite or history entries are clicked before the search cooldown ends

•  added unit price display to listings tab

•  added French translation (a big thanks to Ayantir!)

•  updated German translation


version 0.22


•  fixed listing count not being updated when an item is canceled

•  added sales info to mails


version 0.21


•  updated russian localization (thanks to KiriX)

•  hid stolen items on sell tab as they cannot be sold anyways

•  fixed micro freeze on category change

•  fixed price range not getting restored from history or favorites

•  added missing subcategory for containers to consumables category

•  updated LibAddonMenu-2.0 to r18


version 0.20
This release contains some fundamental changes to the code. If you notice anything that does not work as expected, please let me know in the comment section.


• moved most of the startup code into separate lua files

• updated filter state serialization to allow other addons to save their custom filter states

• rewrote most of the filter code to be more generic

• implemented dynamic filter and button layout on search tab

• removed some settings that are no longer available

• added Russian localization (thanks to KiriX)

• added loading overlay to listings tab

• added sort headers to listings tab

• improved text filter on search tab (see addon description for details)

• added known recipe and motif filters

• added visual difference between normal and local filters (bluish colored label and a descriptive tooltip on mouse over)

• fixed last selected guild name being overwritten when you visit a kiosk

• fixed many other things that nobody ever noticed ;)


version 0.19.1


• updated libFilters to r14

• changed when libFilters are initialized (this may or may not help with some crashes)

• fixed a case where the "no results" label showed up while the show previous page button was visible


version 0.19


• updated to latest API version (100011 / update 6)

• Note: This release is backwards compatible to update 5, but it will be shown as outdated in the addon manager

• updated LibAddonMenu-2.0 to r17

• allow changing the sort order without triggering a search when no results are shown

• added option to always allow changing the sort order without triggering a search

• added feature to remember the last selected sort order in the search tab (you can disable it in the settings)

• Note: In Update 5 disabling it in settings might not work as expected

• added "show previous page" entry to search result list

• reset subcategory when clicking on the currently selected category button in search and sell tab

• always show the Guild Tabard when the Appearance subcategory button is clicked, even when it was already active

• Note: This will clear the shown search results

• select items in the sell tab with a single click (can be disabled in settings)

• added option to disable tooltips with details about an entry in the search library

• added tooltip that shows the currently hired trader of a guild when you hover over the guild name

• Note: Only works when you access the store from a bank or an owned trader

• Note: In update 5 this does not show the tooltip when you hover over a guild name in the dropdown menu

•  fixed incorrect guild order when switching via mouse wheel

•  added context menu to search library with functions to:


• open AGS addon settings directly

• clear history or favorites

• undo the last one deletion (e.g. when you delete both history and favorites you can only undo the deletion of the favorites, so be careful!)

• lock/unlock window size and position

• reset window size and position`;

/** esoui.com id 7 */
export const libAddonMenu = `2.0 r43
- fixed error when playing in XBox Game Pass PC version (thanks DakJaniels)
- fixed leaking global variable (#155, thanks DakJaniels)
- fixed inconsistent behavior in multi select dropdown when using choiceValues (#156, thanks Kyzeragon)
- updated for Seasons Zero

2.0 r42 (consoles only)
- temporarily turned LHAS into an optional dependency, until the situation is resolved. This unfortunately means, there won't be a settings menu for consoles until then.

2.0 r41
- added "maxChars" to HAS conversion for console (#153, thanks M0RGaming)
- fixed dropdown width not getting calculated correctly
- fixed dropdown height no longer being restricted after opening it for the second time
- updated for Seasons of the Worm Cult Part 2

2.0 r40
- added (temporary) LAM to HAS conversion for console (#149, thanks Dolgubon)

2.0 r39
- added "compatibility" for console
- this just makes it so that there are no errors in console flow - actual menu generation will be subject of a future v3
- fixed click sound no longer working in addon list (#147, thanks DakJaniels)
- updated Chinese translation (#144, thanks Jacko9et)
- updated for Seasons of the Worm Cult Part 1

2.0 r38
- fixed submenus only opening once after Update 45 (#145, thanks Baertram)
- added new callback "LAM-PanelControlsCreated" which gets called right before a panel is shown for the first time
- updated for Fallen Banners

2.0 r37
- added Turkish and Ukrainian translations (#137, thanks Sharlikran)
- fixed multi-select dropdowns not showing selected entries correctly (#140, thanks MycroftJr)
- fixed dropdown choice tooltips not working correctly (thanks Calamath)
- updated for Gold Road

2.0 r36
- added multiselect feature to dropdown control (#135, thanks Baertram)
- fixed anchor constraint warnings in the interface.log (#136, thanks DakJaniels)
- fixed a bug which could lead to some controls not getting created in some rare cases
- updated for Scions of Ithelia

2.0 r35
- added "resetFunc" to each control type which gets called while resetting a panel to default values (#130, thanks Baertram)
- added workaround for dropdown menus getting cut off when used inside submenus
- updated for Secret of the Telvanni

2.0 r34
- added tooltips for header and description controls (#129, thanks remosito)
- fixed old icons not being hidden when choices are updated on the icon picker (thanks Gandalf)
- updated for High Isle

2.0 r33
- fixed dropdown widget choicesValues not accepting boolean "false" (#127, thanks Baertram)
- switched to a new build system
- updated for Ascending Tide

2.0 r32
- added "createFunc", "minHeight" and "maxHeight" properties to custom control (#123, thanks Baertram)
- updated folder structure (#119)
- updated for Markarth

2.0 r31
- fixed iconpicker showing an empty tooltip when no choicesTooltips are set (#111, thanks Scootworks)
- fixed slider mouse wheel interactions (#115)
- fixed translated texts not showing in the official Russian localization (#118, thanks andy.s)
- improved dropdown choice tooltip code compatibility (#115)
- added "helpUrl" property for many control types (#109, thanks Baertram)
- added "textType" and "maxChars" properties for editbox (#110, thanks Scootworks)
- added "readOnly" property for slider (#112, thanks Scootworks)
- removed embedded copy of LibStub (#116)
- updated Japanese translation (#113, thanks Calamath)
- updated for Greymoor

2.0 r30
- updated Korean translation (thanks whya5448)
- added "enableLinks" property to description control (#102, thanks silvereyes333)
- updated for Dragonhold

2.0 r29
- fixed a rare error when a panel refresh is triggered by an addon before LAM is fully initialized (#98)
- fixed SetHandler warning showing when a scrollable dropdown is used (#97)
- improved SetHandler warning message to show the panel title instead of the internal name and in addition log to LibDebugLogger for easy access to a stack trace (#99)
- improved comments in control files (#100, thanks Phuein)
- adjusted ReloadUI warning color to match the color of the warning in the ingame video settings (#101, thanks Phuein)

2.0 r28
- fixed color picker throwing errors in gamepad mode (#94, thanks Gandalf)
- added global variable "LibAddonMenu2" for direct access without using LibStub (#95)
- added IsLibrary directive to manifest (#93)
- added warning message when an addon is setting the "OnShow", "OnEffectivelyShown", "OnHide" or "OnEffectivelyHidden" handler on a panel (#92)
- use the callbacks "LAM-PanelControlsCreated", "LAM-PanelOpened" and "LAM-PanelClosed" instead
- updated Brazilian translation (thanks FelipeS11)

2.0 r27
- fixed scrollable dropdown not working correctly (#83)
- fixed disabled sliders changing value in some situations when clicked
- fixed panel not refreshing on open when it was already selected (#82)
- added RefreshPanel function to panel control (#84)
- the panel control is returned by RegisterAddonPanel
- added "translation", "feedback" and "donation" properties to panel (#88, thanks Baertram)
- all three (and also the "website" property) accept a function or a string
- added "disabled" and "disabledLabel" property for submenus (#86, #90, thanks klingo)
- added "icon" and "iconTextureCoords" property for submenus (#91)
- added "disabled" property for descriptions (#89, thanks klingo)
- added "clampFunction" property for slider controls (#85)
- the function receives the value, min and max as arguments and has to return a clamped value
- added optional support for LibDebugLogger
- in case it is loaded, it logs the full error when control creation failed
- updated LibStub to r5

2.0 r26
- fixed error when loading LAM on an unsupported locale
- added Korean translation (thanks p.walker)
- added Brazilian translation (thanks mlsevero)

2.0 r25
- fixed tooltips not working for entries in scrollable dropdown controls (#78, thanks kyoma)
- fixed standalone LAM not loading as expected when LAM is bundled with the manifest included (#81)
- fixed slashcommands not opening the correct panel on first attempt after UI load (#79)`;

/** esoui.com id 2528 */
export const libCombat = `89

•  Add tracking for Nightblade class mastery to enable higher critical damage ceiling in stats.


88

•  Fix a issue where status effect changes were not handled by the combat log. (Thx DakJaniels for providing the fix)


87

•  Fix a lua error occuring when no Subclassing is not available, e.g. Vengeance Campaign (thx Masteroshi430 for the report)

•  Improve boss fight detection (thx Charles for the report)


86

•  Added additional info to charData to support CMX build export


85

•  Fixed missing formatting of ability names in GetFormattedAbilityName (thx Baertram for the report)

•  Improved fight separation for dueling (thx wjtk4444 for the report) .

•  Fix sprint detection for Resources tracking

•  Fix detection of Arcanist passive for status effecct bonus (thx GdeMoiKrendelki for the report)


84

•  Fixed an issue where incorrect fight recap group numbers were given for the callback in rare cases.


83

•  Minor fixes and improvements.


82

•  Fixed an issue, where casts of Ulfsild's Contingency were not properly tracked. (Thx to Skinny Cheeks for reporting)


81

•  Fixed an issue causing a LUA error when using the Symmetry of the Weald set. (Thx to arkoni and realm87 for reporting this)


80

•  Bring back a previously removed entry in fight recap callback ("HPSAOut") for backwards compatibility.


79

•  Fix status effect tracking for werewolves. (Thx to Paduraschka for reporting)


78

•  Fix description parsing for various non-english languages. (Thanks for everyone reporting)


77

•  Added some checks to prevent error messages.


76

•  Add tracking of status effect proc chance

•  Add lookup tables for food & drink buffs and mundus stones

•  Add tracking of quick slot actions

•  Add additional data to fight recap callback: player damage & group damage to boss units

•  Some refactoring


75

•  Added a parameter to LIBCOMBAT_EVENT_SKILL_TIMINGS callback to allow a fix for SimpleCastBar.


74

•  Added a fix for a lua error on Update 43. (Thx to code65536 for the report and fix)


73

•  Added support for some Update 42 changes. (Thx to Anthonysc)


72

•  Fixed an error due to removed constants


71

•  Fixed tracking of Pulsar. (Thx to isiiimode for reporting)

•  Minor Fixes


70

•  Fixed tracking of several Arcanist abilities (incl. Fulminating Rune) when main resource is Stamina (Thx to Helix and brdalert for reporting this).


69

•  Reverted some code that was not ready for publishing


68

•  Added Support for weaving tracking of Arkanist Skill Lines

•  Fixed and improved tracking of resources.`;

/** esoui.com id 57 */
export const harvestMap = `## 3.16.12
- fixed lua error when using heatmap mode

## 3.16.11
- fixed lua error when collecting herbalist's satchel

## 3.16.10
- fixed error when playing in German

## 3.16.9
- fixed error when harvesting jewelry ore

## 3.16.8b
- internal changes to improve data exchange server efficiency.

## 3.16.8
- fixed interaction target not being detected (which caused incorrect resource location being recorded)
- fixed pins for "unknown nodes" being displayed despite the node actually being known already
- updated german localization

## 3.16.7b
- actually uploaded the new files... whoops

## 3.16.7
- fixed issue where available pin filters would not update when discovering crimson nirnroot, stashes, herbalist satchels for the first time on a map (2nd attempt...)
- added option to change the 3d pin base texture
- added 3d pin base to the texture presets
- fixed spawned filter not working on the main map when "show only nearby pins" was disabled

## 3.16.6
- fixed issue where available pin filters would not update when discovering crimson nirnroot, stashes, herbalist satchels for the first time on a map
- added color and texture presets. now addons like HarvestPins can change the default icons.

## 3.16.5
- fixed nodes not being displayed on the solstice map if the player character is currently in solstice

## 3.16.4
- maps that change from one version to another will have their data merged (eg solstice)
- If you downloaded node locations, then you will need to redownload them again.

## 3.16.3
- fixed compass distances
- collected data from old solstice map will be displayed (this applies your locally collected data. i still need to fix the downloadable data on the server)

## 3.16.2
- added spanish localization by cisneros
- fixed lua error that could occur when entering subzones/cities while displaying unknown resource pins

## 3.16.1
- fixed lua error when no data is available for the current map

## 3.16.0
- greatly improved performance on maps with many nodes.
(had to remove the visited timer for this)
- added map filters to console UI
- added herbalist satchel pin type (eg blackreach)
- pin type filters now depend on the current map to keep list shorter (eg herbalist satchel filter checkbox is only displayed on maps that have herbalist satchels)
- added scrollable map filters when filter list gets too long

## 3.15.14
fixed stuck 3d pins that can occur while the respawn filter is enabled

## 3.15.13
fixed crash on PTS

## 3.15.12
added french localization by XXXspartiateXXX

## 3.15.11
removed a debug message that was printed when opening chests

## 3.15.10
- heavy sacks etc should now be properly saved when playing in german, french, russian, spanish
- fixed various bugs in Blackreach. old pins will be missing on minimap, compass and 3d world, but new pins will be properly displayed. the filter that displays only spawned resources should also work in blackreach
- fixed tour editor on pts server

## 3.15.9
- fixed duplicate name incompatibility with other addons

## 3.15.8
- updated api version
- improved compatibility with old/outdated addons that use outdated versions of LibAddonMenu
- updated russian localization by OlegKRS
- added chinese localization by juijote
- updated german localization

## 3.15.7
- fixed a bug where the data of the AD, DC, EP, DLC or NF module were not loaded until doing a /reloadui

## 3.15.6
- based on several comments in the comment section, the menu for creating filter profiles was too confusing. So I removed the filter profiles from the addon settings. In its place there are now separate lists to turn on/off pin types in the map, compass and 3d pin sections of the addon settings.
(for those that want to use the filter profiles, you can still access that menu via keybind)
- fixed pins not scaling based on the pin scale settings used by Fyrakin's minimap and Votan's minimap
- fixed an issue that caused missing pins on the minimap for chests and safeboxes in solitude and blackreach. (The fix is not retroactive. The pins will be displayed after unlocking chests/safeboxes)

## 3.15.5
- added the option to hide visited pins again
- fixed a bug where 3d pins would not display in cities etc until you open the map
- moved the downloaded data to a separate addon, so the data is no longer deleted with every update to HarvestMap.`;

/** esoui.com id 1292 */
export const alignGrid = `1/22/2024 Version 1.4.4
- Removed LICENSE file

1/21/2024 Version 1.4.3
- Updated APIVersion to 101040

11/4/2022 Version 1.4.2
- Updated APIVersion to 101036

10/29/2022 Version 1.4.1
- Added ability to set a keybind toggle on/off for the grid

10/23/2022 Version 1.4.0
- Updated APIVersion to 101035
- Licensing updates
- Minor bug fixes
- Comment changes
- Updated LibAddonMenu dependency
- Removed LibStub dependency

6/1/2018 Version 1.3.3
- Updated LibAddonMenu to latest version
- Updated APIVersion to 100023

2/16/2018 Version 1.3.2
- Updated APIVersion to 100022

10/6/2017 Version 1.3.1
- Updated APIVersion to 100021
- Updated LibAddonMenu to newest version

8/19/2017 Version 1.3.0
- Updated APIVersion to 100020
- Updated LibAddonMenu to newest version

5/29/2017
- Updated APIVersion to 100019

2/7/2017 Version 1.2.8
- Updated APIVersion to 100018

12/14/2016 Version 1.2.7
- Added ZOS License to main file
- Added missing library file

11/13/2016 Version 1.2.6
- Updated APIVersion to 100017
- Updated LibaddonMenu to newest version
- Updated LibStub to newest version

3/9/2016 Version 1.2
- Fixed folder structure so the addon now works...I'm sorry!
- Cleaned up some code so it shouldn't break the "Addons Settings Menu" anymore

3/7/2016 Version 1.1
- Updated APIVersion to 100014
- Updated LibAddonMenu to newest version`;

/** esoui.com id 1456 */
export const keybindingLogOut = `• Version 1.0.18 (2021/03/08)

• API version bump for Update 29 (Flames of Ambition)




• Version 1.0.17 (2020/11/05)

• API version bump for Update 28 (Markarth)




• Version 1.0.16 (2020/08/26)

• API version bump for Update 27 (Stonethorn)




• Version 1.0.15 (2020/05/26)

• API version bump for Update 26 (Greymoor)




• Version 1.0.14 (2020/02/05)

• API version bump for Update 25 (Harrowstorm)




• Version 1.0.13 (2019/10/23)

• API version bump for Update 24 (Dragonhold)




• Version 1.0.12 (2019/08/20)

• API version bump for Update 23 (Scalebreaker)




• Version 1.0.11 (2019/05/20)

• API version bump for Update 22 (Elsweyr)




• Version 1.0.10 (2019/03/21)

• Minor name change for consistency with other keybinding addons




• Version 1.0.9 (2019/02/25)

• API version bump for Update 21 (Wrathstone)




• Version 1.0.8 (2018/10/22)

• API version bump for Update 20 (Murkmire)




• Version 1.0.7 (2018/08/12)

• API version bump for Update 19 (Wolfhunter)

• Changed keybinding category from "General > User Interface" to "General > General"




• Version 1.0.6 (2018/05/28)

• API version bump for Update 18 (Summerset)




• Version 1.0.5 (2018/02/12)

• API version bump for Update 17 (Dragon Bones)




• Version 1.0.4 (2017/10/23)

• API version bump for Update 16 (Clockwork City)




• Version 1.0.3 (2017/08/14)

• API version bump for Update 15 (Horns of the Reach)




• Version 1.0.2 (2017/05/22)

• API version bump for Update 14 (Morrowind)




• Version 1.0.1 (2016/10/05)

• API version bump for Update 12 (One Tamriel)




• Version 1.0.0 (2016/08/14)

• Initial version`;

/** esoui.com id 93 */
export const pChat = `- Maintained by Baertram. Please do not ask for new features. I'll just keep the addon running. Thanks for your support -


Changelog:
## v10.0.7.4 ## 2026-06-08
Udated API version and dependencies
Fixed usage of old chat variable (in history restore)


## v10.0.7.3 ## 2026-04-14
Added Chinese translation, thanks to lolo77777

## v10.0.7.2 ## 2026-01-21
Fixed 
#37 Chat Mentions are not checking guild advertisements now
#38 PTS Season Zero fix to support keybindings chording (SHIFT, CTRL, ALT key in addition)

## v10.0.7.1 ## 2025-11-28
Fixed:
-#36 Errors at /pchats chat search slash command (thanks to Dakjaniels)
-Fixed some translation errors
-Added more description to the Chat mentions' settings, and changed some texts to be more clear where they belong to (in case you do not read the tooltips at the controls, which always is the best solution ;-))
-Updated the LAM sound slider widget for chat mentions (please update the library to version 6!)

## v10.0.7.0 ## 2025-11-23
-Fixed version
-Fixed some fonts not rendering fine in Chat
-Added font preview to font selection dropdown (above it, at the settings menu)
If you change the font dropdown now the UI won't reload directly! You can preview the fonts above the dropdown and apply the reload UI manually as you did change the settings you want to change (uses LAM "Settings were changed at panel, do you want to reload the UI now?" dialog)

## v10.0.6.9 ## 2025-11-23
-Updated dependencies and API version.
-Fixed some smaller issues and an issue with fonts loading (thanks Dakjaniels!).
-Added a "Chat is scrolled up" warning setting to "Chat settings" -> "Chat window settings". Once enabled (default: OFF) your chat "Scroll to bottom" button will ping-pong with the size if you scrolled the chat up (and thus might miss any new incoming messages due to that).
-Added a dropdown for the accounts to the chat copy dialog (top left) where you can select the @account to search the chat for (if you play with multiple accounts).
The dropdown's @accountName can be preset in the settings "Restore chat" -> "Default account (Chat search)".
The dropdown is only selectable if you are searching the whole chat (not for single messages).
The chose @accountName from that dropdown will filter the chat messages at the chat copy dialog and the search UI!

## v10.0.6.8 ## 2025-08-17
-Updated APIversion for 101047
-Added slash command for the chat search:
/pchats 

The chat search is based on the "copy chat" dialog, which is opend from the context menu as you click the name/timestamp at the chat. Make sure to enable the chat history if you want to search old chat too (up to 24h only! Everything more would explode the SavedVariables and make all loading screens slooooooow).

## v10.0.6.7 ## 2025-07-03
-Fixed slash commands for teleport to group, guild friends: Do not port to yourself
-Fixed slash command for teleport to guild member /tpg to accept either the guildIndex first and then the @displayname or charactername or parts of it only (1st partially matching member who is online will be teleported to)

## v10.0.6.6 ## 2025-06-12
--Dependency LibMediaProvider (renamed library)

## v10.0.6.5 ## 2025-06-08
-Updated ES translation, thanks to MasterZiggy
-Fixed system chat channel not showing at chat tab configuration
-Updated APIversion and dependencies

## v10.0.6.4 ## 2025-02-07
-Fixed French translations and version
-Fixed a bug with collectable items in itemlinks

## v10.0.6.2 + 3 ## 2025-01-26
-Fixed /msg add/remove updating the LibSlashCommander auto completion for the chat mesages started by !

## v10.0.6.1 ## 2025-01-25
-Fix ChatTabChannelSwitch
-Added pChat.lastCheckDisplayNameData for other addons to get the currently right clicked @displayName e.g. from the chat player context menu (if pChat setting to teleport to players via context menu is enabled)

## v10.0.6.0 ## 2025-01-05
Fixed:
-Some wrong translations and missing translations

Changed:
-Settings menu for colors describes better why settings are disabled and where to change it (submenus are disabled in total e.g.) 

Added:
-Timestamps with milliseconds format. Use xy in the formatting string to activate millisenconds (thanks to Dakjaniels for the idea and code)
-Auto completion for /msg: Activate LibSlashCommander and type ! in the chat to show your defined ! messages, and their text`;

/** esoui.com id 1121 */
export const craftingWritAssistant = `CHANGE LOG:


• 08-30-2020 Enabled selected writs to remain selected after crafting, Updated recipe name check to use zo_strformat (thanks Baertram)

• 07-17-2020 Removed lib folder and added needed Libs to "dependenies"

• 04-12-2020 Removed Libstub reference

• 03-08-2020 Added option for leaving window open when exiting station, must be enabled in add-on settings

• 07-21-2019 Updated UI for Enchanting and Provision Details to be dynamic based on the quest step info

• 07-20-2019 Updated RU translation file, Added Enchanting Rune names for Enchanting writs

• 05-27-2018 Added Jewelry crafting writ support.

• 02-25-2018 Fixed a UI Display bug when turning in a Master Writ.

• 02-19-2018 Added Completion Text Color indicator  (Thanks to DonRomano for doing this work)

• 01-28-2018 Updated API for Dragon Bones

• 01-25-2018 Added New Setting to display window at Guild Store

• 01-16-2018 Fixed Display issue where the Writ window was not displayed due to zoning bug.

• 01-11-2018 Fixed Display issue for Second Provisioning Recipe not displaying ingredients.

• 12-31-2017 Updated API for Clockwork City, Added Master Writ Support, Expanded German translations support

• 09-04-2017 Updated API for Horns of the Reach, and added bug fixes

• 08-02-2016 Updated API for Shadows of the Hist

• 04-30-2016 Updated API for Thieves Guild

• 02-25-2016 Added German translations for writs (Special Thanks to jr.returns)

• 09-04-2015 Update to new API for Imperial City update, updated quest log when items are pulled from bank or guild bank

• 08-04-2015 added new settings for guild bank display, bank display

• 06-04-2015 Added new option to display provisioning ingredients next to writs

• 03-04-2015 fixed window setting to remember your preference (Special thanks Phinix)`;
